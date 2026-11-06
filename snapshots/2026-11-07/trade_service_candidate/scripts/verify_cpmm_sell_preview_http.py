#!/usr/bin/env python3
"""Exercise the SDK 11 read-only sell preview in a disposable service."""

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
BINARY = Path(os.environ.get('CANDIDATE_PREVIEW_BINARY', ''))
SOURCE_WALLETS = Path(os.environ.get(
    'CANDIDATE_PREVIEW_WALLET_DB', '/tmp/candidate-staging-data/wallets.sqlite',
))


def request(url, token=None, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {'content-type': 'application/json'}
    if token:
        headers['authorization'] = f'Bearer {token}'
    try:
        with urlopen(Request(url, data=data, headers=headers), timeout=30) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else None
    except HTTPError as error:
        raw = error.read()
        try:
            result = json.loads(raw) if raw else None
        except json.JSONDecodeError:
            result = raw.decode(errors='replace')
        return error.code, result


def main():
    if not os.environ.get('CANDIDATE_PREVIEW_BINARY') or not BINARY.is_file() or not SOURCE_WALLETS.is_file():
        raise FileNotFoundError('set CANDIDATE_PREVIEW_BINARY to an isolated candidate binary and provide a wallet database')
    if not os.environ.get('RPC_URL') or not os.environ.get('WALLET_DATABASE_PASSPHRASE_FILE'):
        raise ValueError('source candidate .env before running this verifier')
    with tempfile.TemporaryDirectory(prefix='sdk11-sell-preview-') as temp:
        work = Path(temp)
        with sqlite3.connect(f'file:{SOURCE_WALLETS}?mode=ro', uri=True) as source:
            with sqlite3.connect(work / 'wallets.sqlite') as destination:
                source.backup(destination)
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = 'sdk11-disposable-preview-token'
        env = dict(os.environ,
                   TRADE_HTTP_ADDR=f'127.0.0.1:{port}',
                   TRADE_DATABASE_URL=f'sqlite://{work / "trades.sqlite"}',
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
        with (work / 'service.log').open('w') as log:
            service = subprocess.Popen([str(BINARY)], cwd=ROOT, env=env,
                                       stdout=log, stderr=subprocess.STDOUT)
            try:
                for _ in range(240):
                    if service.poll() is not None:
                        raise RuntimeError('disposable candidate exited before health')
                    try:
                        with urlopen(base + '/health', timeout=2) as response:
                            if response.status == 200:
                                break
                    except (OSError, URLError):
                        time.sleep(0.25)
                else:
                    raise RuntimeError('disposable candidate did not become healthy')
                path = base + '/v1/routes/cpmm-sell-preview'
                body = {
                    'wallet_id': os.environ.get('CANDIDATE_PREVIEW_WALLET_ID', '2TE'),
                    'target_pool_id': os.environ.get('CANDIDATE_PREVIEW_TARGET_POOL_ID',
                        'gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U'),
                    'bridge_pool_id': os.environ.get('CANDIDATE_PREVIEW_BRIDGE_POOL_ID',
                        'fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2'),
                    'target_mint': os.environ.get('CANDIDATE_PREVIEW_TARGET_MINT',
                        '3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG'),
                    'quote_mint': os.environ.get('CANDIDATE_PREVIEW_QUOTE_MINT',
                        'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
                    'sell_percent': int(os.environ.get('CANDIDATE_PREVIEW_SELL_PERCENT', '10')),
                    'slippage_bps': int(os.environ.get('CANDIDATE_PREVIEW_SLIPPAGE_BPS', '100')),
                }
                ready_status, ready = request(base + '/ready', token)
                assert ready_status == 200, ready_status
                wallet = next(item for item in ready['wallets']
                              if item['wallet_id'] == body['wallet_id'])
                rpc_request = Request(
                    os.environ['RPC_URL'],
                    data=json.dumps({
                        'jsonrpc': '2.0', 'id': 1,
                        'method': 'getTokenAccountsByOwner',
                        'params': [wallet['pubkey'], {'mint': body['target_mint']},
                                   {'encoding': 'jsonParsed'}],
                    }).encode(),
                    headers={'content-type': 'application/json'},
                )
                try:
                    with urlopen(rpc_request, timeout=20) as response:
                        rpc_result = json.load(response)
                    wallet_target_account_count = len(rpc_result['result']['value'])
                except (KeyError, OSError, URLError):
                    wallet_target_account_count = None
                unauthorized, _ = request(path, body=body)
                assert unauthorized in (401, 403), unauthorized
                unknown_wallet, _ = request(path, token, {**body, 'wallet_id': 'absent'})
                assert unknown_wallet == 400, unknown_wallet
                extra_field, _ = request(path, token, {**body, 'simulate': False})
                assert extra_field == 422, extra_field
                same_pool, _ = request(path, token, {
                    **body, 'bridge_pool_id': body['target_pool_id'],
                })
                assert same_pool == 502, same_pool
                valid_route, result = request(path, token, body)
                assert valid_route in (200, 502), valid_route
                if wallet_target_account_count == 0:
                    assert valid_route == 502, valid_route
                unsigned_simulation, simulated = request(path, token, {
                    **body, 'unsigned_simulation': True,
                })
                if wallet_target_account_count == 0:
                    assert unsigned_simulation == 502, unsigned_simulation
                if valid_route == 200:
                    assert result['executable'] is False
                    assert result['preview']['executable'] is False
                    assert result['simulate_only'] is True
                    assert 'sdk_decision_id' not in result and 'preview_id' not in result
                if os.environ.get('CANDIDATE_PREVIEW_REQUIRE_SUCCESS') == '1':
                    assert valid_route == 200 and unsigned_simulation == 200
                    assert simulated['preview']['simulation']['balance_and_minimum_verified'] is True
                    assert simulated['preview']['simulation']['ok'] is True
                    assert simulated['preview']['simulation']['rpc_accepted'] is True
                    assert simulated['preview']['simulation']['input_token_debit_units'] \
                        == simulated['preview']['input_token_units']
                    assert simulated['preview']['simulation']['wsol_credit_units'] \
                        >= simulated['preview']['bridge_min_wsol_units']
                with sqlite3.connect(work / 'trades.sqlite') as database:
                    counts = {name: database.execute(f'SELECT COUNT(*) FROM {name}').fetchone()[0]
                              for name in ('trades', 'aggregator_quotes', 'aggregator_submissions')}
                assert counts == {name: 0 for name in counts}, counts
                print(json.dumps({
                    'unauthorized_http': unauthorized,
                    'unknown_wallet_http': unknown_wallet,
                    'extra_field_http': extra_field,
                    'same_pool_http': same_pool,
                    'valid_route_http': valid_route,
                    'unsigned_simulation_http': unsigned_simulation,
                    'preview_built': valid_route == 200,
                    'wallet_target_account_count': wallet_target_account_count,
                    'missing_target_account_closed': wallet_target_account_count == 0 and valid_route == 502,
                    'input_token_units': simulated['preview']['input_token_units']
                        if unsigned_simulation == 200 else None,
                    'input_token_debit_units': simulated['preview']['simulation']['input_token_debit_units']
                        if unsigned_simulation == 200 else None,
                    'wsol_credit_units': simulated['preview']['simulation']['wsol_credit_units']
                        if unsigned_simulation == 200 else None,
                    'minimum_wsol_units': simulated['preview']['bridge_min_wsol_units']
                        if unsigned_simulation == 200 else None,
                    'balance_and_minimum_verified': simulated['preview']['simulation']['balance_and_minimum_verified']
                        if unsigned_simulation == 200 else None,
                    'transaction_version': simulated['preview']['simulation']['transaction_version']
                        if unsigned_simulation == 200 else None,
                    'transaction_bytes': simulated['preview']['simulation']['transaction_bytes']
                        if unsigned_simulation == 200 else None,
                    'record_counts': counts,
                    'signed_or_submitted': False,
                }, sort_keys=True))
            finally:
                service.terminate()
                try:
                    service.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    service.kill()
                    service.wait(timeout=5)


if __name__ == '__main__':
    main()
