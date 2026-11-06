#!/usr/bin/env python3
"""Exercise a CPMM->CPMM buy on a disposable simulate-only Trade Service."""

import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import URLError
from urllib.request import urlopen

from verify_cpmm_sell_order_http import ROOT, counts, request


def main():
    binary = Path(os.environ.get('CANDIDATE_BUY_ORDER_BINARY', ''))
    wallets = Path(os.environ.get('CANDIDATE_PREVIEW_WALLET_DB',
                                  '/tmp/candidate-staging-data/wallets.sqlite'))
    if not os.environ.get('CANDIDATE_BUY_ORDER_BINARY') or not binary.is_file():
        raise FileNotFoundError('set CANDIDATE_BUY_ORDER_BINARY to an isolated binary')
    if not wallets.is_file() or not os.environ.get('RPC_URL'):
        raise FileNotFoundError('source candidate .env and provide wallet DB')
    with tempfile.TemporaryDirectory(prefix='sdk11-cpmm-buy-order-') as name:
        work = Path(name)
        with sqlite3.connect(f'file:{wallets}?mode=ro', uri=True) as original:
            with sqlite3.connect(work / 'wallets.sqlite') as copied:
                original.backup(copied)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = 'disposable-cpmm-buy-order-token'
        db = work / 'trades.sqlite'
        env = dict(os.environ,
                   TRADE_HTTP_ADDR=f'127.0.0.1:{port}',
                   TRADE_DATABASE_URL=f'sqlite://{db}',
                   WALLET_DATABASE_URL=f'sqlite://{work / "wallets.sqlite"}',
                   CONVERSION_DATABASE_URL=f'sqlite://{work / "conversions.sqlite"}',
                   TRADE_PERSISTENCE_MODE='durable',
                   WALLET_STORAGE_MODE='database',
                   TRADE_SERVICE_SIMULATE_ONLY='true',
                   SDK_TWO_HOP_LIVE_ENABLED='false',
                   ENABLE_SIMULATE='true',
                   CONVERSION_LIVE_ENABLED='false',
                   CONVERSION_QUOTES_ENABLED='false',
                   ENABLE_PUMPAPI_EXECUTOR='false',
                   LOCAL_TRADE_API_TOKEN=token,
                   RUST_LOG='warn')

        def start():
            log = (work / 'service.log').open('a')
            service = subprocess.Popen([str(binary)], cwd=ROOT, env=env,
                                       stdout=log, stderr=subprocess.STDOUT)
            for _ in range(240):
                if service.poll() is not None:
                    log.close()
                    raise RuntimeError('disposable candidate exited before health')
                try:
                    with urlopen(base + '/health', timeout=2) as response:
                        if response.status == 200:
                            return service, log
                except (OSError, URLError):
                    time.sleep(0.25)
            service.terminate()
            log.close()
            raise RuntimeError('disposable candidate did not become healthy')

        def stop(service, log):
            service.terminate()
            try:
                service.wait(timeout=5)
            except subprocess.TimeoutExpired:
                service.kill()
                service.wait(timeout=5)
            log.close()

        trade = {
            'wallet_id': os.environ.get('CANDIDATE_BUY_WALLET_ID', '2TE'),
            'side': 'buy',
            'pool': 'raydium-cpmm',
            'mint': os.environ.get('CANDIDATE_BUY_MINT',
                                 '3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh'),
            'pool_id': os.environ.get('CANDIDATE_BUY_TARGET_POOL_ID',
                                    'bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e'),
            'quote_mint': os.environ.get('CANDIDATE_BUY_QUOTE_MINT',
                                       'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB'),
            'settlement_asset': 'sol',
            'sol_hop_pool': 'raydium-cpmm',
            'sol_hop_pool_id': os.environ.get('CANDIDATE_BUY_BRIDGE_POOL_ID',
                                            'AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk'),
            'amount_lamports': int(os.environ.get('CANDIDATE_BUY_LAMPORTS', '1000000')),
            'slippage_bps': 100,
            'execution_method': 'sdk',
            'simulate': True,
            'idempotency_key': f'cpmm-buy-disposable-{os.getpid()}',
        }
        service, log = start()
        try:
            no_decision, _ = request(base + '/v1/trades', token, trade)
            assert no_decision == 400, no_decision
            assert counts(db)['trades'] == 0
            decision_status, decision = request(base + '/v1/routes/decide', token, trade)
            assert decision_status == 200 and decision['status'] == 'sdk_verified', decision
            assert decision['sdk_capability']['service_adapter'] == 'cpmm_two_hop_buy_simulate_only'
            assert decision['sdk_capability']['route_evidence'] == 'unsigned_simulation_verified'
            bound = {**trade, 'sdk_decision_id': decision['decision_id']}
            rejected = {}
            for label, changes in {
                'amount': {'amount_lamports': trade['amount_lamports'] + 1},
                'slippage': {'slippage_bps': 500},
                'bridge': {'sol_hop_pool_id': trade['pool_id']},
                'wallet': {'wallet_id': 'main'},
                'live': {'simulate': False},
            }.items():
                status, _ = request(base + '/v1/trades', token, {**bound, **changes})
                assert status in (400, 403), (label, status)
                rejected[label] = status
            assert counts(db)['trades'] == 0
            status, order = request(base + '/v1/trades', token, bound)
            assert status in (200, 202), (status, order)
            order_id = order['request_id']
            for _ in range(120):
                _, order = request(base + f'/v1/trades/{order_id}', token)
                if order['status'] not in ('accepted', 'warming_params', 'building_tx', 'submitting'):
                    break
                time.sleep(0.25)
            assert order['status'] == 'simulated' and order['signature'] is None, order
            assert order['simulation']['source'] == 'sdk-11:cpmm-two-hop-buy', order
            retry_status, retry = request(base + '/v1/trades', token, bound)
            assert retry_status == 200 and retry['request_id'] == order_id and retry['duplicate']
            assert counts(db) == {'trades': 1, 'aggregator_quotes': 0,
                                  'aggregator_submissions': 0}
        finally:
            stop(service, log)
        service, log = start()
        try:
            status, readback = request(base + f'/v1/trades/{order_id}', token)
            assert status == 200 and readback['status'] == 'simulated'
            assert readback['signature'] is None
            status, retry = request(base + '/v1/trades', token, bound)
            assert status == 200 and retry['request_id'] == order_id and retry['duplicate']
            final_counts = counts(db)
            assert final_counts == {'trades': 1, 'aggregator_quotes': 0,
                                    'aggregator_submissions': 0}
            print(json.dumps({'decision': decision['status'], 'order': readback['status'],
                              'no_decision_http': no_decision, 'rejected_http': rejected,
                              'signature': readback['signature'], 'restart_readback': True,
                              'records': final_counts}, sort_keys=True))
        finally:
            stop(service, log)


if __name__ == '__main__':
    main()
