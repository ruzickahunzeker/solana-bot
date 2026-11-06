#!/usr/bin/env python3
"""Verify a wallet-bound SDK 11 CPMM sell through a disposable order service."""

import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get('CANDIDATE_SELL_ORDER_BINARY', ''))
SOURCE_WALLETS = Path(os.environ.get(
    'CANDIDATE_PREVIEW_WALLET_DB', '/tmp/candidate-staging-data/wallets.sqlite',
))


def request(url, token, body=None, timeout=90):
    data = None if body is None else json.dumps(body).encode()
    headers = {'authorization': f'Bearer {token}', 'content-type': 'application/json'}
    try:
        with urlopen(Request(url, data=data, headers=headers), timeout=timeout) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else None
    except HTTPError as error:
        raw = error.read()
        try:
            result = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            result = raw.decode(errors='replace')
        return error.code, result


def counts(database):
    with sqlite3.connect(database) as con:
        return {name: con.execute(f'SELECT COUNT(*) FROM {name}').fetchone()[0]
                for name in ('trades', 'aggregator_quotes', 'aggregator_submissions')}


def main():
    if not os.environ.get('CANDIDATE_SELL_ORDER_BINARY') or not BINARY.is_file():
        raise FileNotFoundError('set CANDIDATE_SELL_ORDER_BINARY to an isolated candidate binary')
    if not SOURCE_WALLETS.is_file() or not os.environ.get('RPC_URL'):
        raise FileNotFoundError('source candidate .env and provide a wallet database copy')
    with tempfile.TemporaryDirectory(prefix='sdk11-cpmm-sell-order-') as name:
        work = Path(name)
        with sqlite3.connect(f'file:{SOURCE_WALLETS}?mode=ro', uri=True) as original:
            with sqlite3.connect(work / 'wallets.sqlite') as copied:
                original.backup(copied)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = 'disposable-cpmm-sell-order-token'
        db = work / 'trades.sqlite'
        env = dict(os.environ,
                   TRADE_HTTP_ADDR=f'127.0.0.1:{port}',
                   TRADE_DATABASE_URL=f'sqlite://{db}',
                   WALLET_DATABASE_URL=f'sqlite://{work / "wallets.sqlite"}',
                   CONVERSION_DATABASE_URL=f'sqlite://{work / "conversions.sqlite"}',
                   TRADE_PERSISTENCE_MODE='durable',
                   WALLET_STORAGE_MODE='database',
                   TRADE_SERVICE_SIMULATE_ONLY='true',
                   ENABLE_SIMULATE='true',
                   CONVERSION_LIVE_ENABLED='false',
                   CONVERSION_QUOTES_ENABLED='false',
                   ENABLE_PUMPAPI_EXECUTOR='false',
                   LOCAL_TRADE_API_TOKEN=token,
                   RUST_LOG='warn')

        def start():
            log = (work / 'service.log').open('a')
            service = subprocess.Popen([str(BINARY)], cwd=ROOT, env=env,
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
            'wallet_id': '2TE',
            'side': 'sell',
            'pool': 'raydium-cpmm',
            'mint': '3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh',
            'pool_id': 'bsitkqbNk5dPgyrVpHRjiAPNQk66xCfNs8wE981Cd8e',
            'quote_mint': 'Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB',
            'settlement_asset': 'sol',
            'sol_hop_pool': 'raydium-cpmm',
            'sol_hop_pool_id': 'AddZXoGEs6pR6bgp51vjNasKN8jFnizsu6j5P3PSphqk',
            'sell_percent': 100,
            'slippage_bps': 100,
            'execution_method': 'sdk',
            'simulate': True,
            'idempotency_key': f'cpmm-sell-disposable-{os.getpid()}',
        }
        service, log = start()
        try:
            no_decision, no_decision_body = request(base + '/v1/trades', token, trade)
            assert no_decision == 400, (no_decision, no_decision_body)
            assert counts(db)['trades'] == 0
            decision_status, decision = request(base + '/v1/routes/decide', token, trade)
            assert decision_status == 200 and decision['status'] == 'sdk_verified', decision
            assert decision['simulate_only'] is True
            assert decision['sdk_capability']['route_evidence'] == 'unsigned_simulation_verified'
            bound = {**trade, 'sdk_decision_id': decision['decision_id']}
            rejected_changes = {
                'sell_percent': {'sell_percent': 50},
                'slippage_bps': {'slippage_bps': 500},
                'bridge_pool_id': {'sol_hop_pool_id': trade['pool_id']},
                'wallet_id': {'wallet_id': 'main'},
                'live': {'simulate': False},
            }
            rejected_statuses = {}
            for label, changes in rejected_changes.items():
                status, _ = request(base + '/v1/trades', token, {**bound, **changes})
                assert status in (400, 403), (label, status)
                rejected_statuses[label] = status
            assert counts(db)['trades'] == 0
            order_status, order = request(base + '/v1/trades', token, bound)
            assert order_status in (200, 202), (order_status, order)
            order_id = order['request_id']
            for _ in range(120):
                _, order = request(base + f'/v1/trades/{order_id}', token)
                if order['status'] not in ('accepted', 'warming_params', 'building_tx', 'submitting'):
                    break
                time.sleep(0.25)
            assert order['status'] == 'simulated' and order['signature'] is None, order
            assert order['simulation']['ok'] is True, order
            assert order['simulation']['source'] == 'sdk-11:cpmm-two-hop-sell', order
            retry_status, retry = request(base + '/v1/trades', token, bound)
            assert retry_status == 200 and retry['request_id'] == order_id, retry
            assert retry['duplicate'] is True
            assert counts(db) == {'trades': 1, 'aggregator_quotes': 0,
                                  'aggregator_submissions': 0}
        finally:
            stop(service, log)
        service, log = start()
        try:
            read_status, readback = request(base + f'/v1/trades/{order_id}', token)
            assert read_status == 200 and readback['status'] == 'simulated'
            assert readback['signature'] is None
            retry_status, retry = request(base + '/v1/trades', token, bound)
            assert retry_status == 200 and retry['request_id'] == order_id
            assert retry['duplicate'] is True
            final_counts = counts(db)
            assert final_counts == {'trades': 1, 'aggregator_quotes': 0,
                                    'aggregator_submissions': 0}
            print(json.dumps({
                'decision_status': decision['status'],
                'decision_simulate_only': decision['simulate_only'],
                'rejected_changes_http': rejected_statuses,
                'no_decision_http': no_decision,
                'order_status': readback['status'],
                'simulation_source': readback['simulation']['source'],
                'signature': readback['signature'],
                'retry_same_order': retry['request_id'] == order_id,
                'restart_readback': True,
                'records': final_counts,
            }, sort_keys=True))
        finally:
            stop(service, log)


if __name__ == '__main__':
    main()
