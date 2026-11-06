#!/usr/bin/env python3
"""Read-only SDK two-hop simulation with an isolated copy of a funded wallet DB."""

import argparse
import json
import os
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def http(base, token, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {'content-type': 'application/json'}
    if token:
        headers['authorization'] = 'Bearer ' + token
    request = urllib.request.Request(base + path, data=data, headers=headers,
                                     method='POST' if body is not None else 'GET')
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def copy_database(source, destination):
    with sqlite3.connect(f'file:{source}?mode=ro', uri=True) as original:
        with sqlite3.connect(destination) as copied:
            original.backup(copied)
    destination.chmod(0o600)


def verify_panel_resolution(base, token, trade, quote_mint):
    status, resolved = http(base, token, '/v1/pools/resolve', {'mint': trade['mint']})
    mint_only_ok = status == 200 and (
        resolved.get('mint'), resolved.get('pool'),
        resolved.get('pool_id'), resolved.get('quote_mint')) == (
            trade['mint'], trade['pool'], trade['pool_id'], quote_mint)
    status, resolved = http(base, token, '/v1/pools/resolve-candidates', {
        'protocol_id': trade['pool'], 'program_id': None,
        'pool_id': trade['pool_id'], 'mint': trade['mint'], 'quote_mint': None,
    })
    assert status == 200, (status, resolved)
    assert resolved['chain_verified'] is True, resolved
    assert (resolved['mint'], resolved['protocol_id'], resolved['pool_id'],
            resolved['quote_mint']) == (
                trade['mint'], trade['pool'], trade['pool_id'], quote_mint), resolved
    return 'mint_only_and_verified_hint' if mint_only_ok else 'verified_hint'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--wallet-db', type=Path, required=True)
    parser.add_argument('--wallet-id', required=True)
    parser.add_argument('--candidate-dir', type=Path, required=True)
    parser.add_argument('--omit-quote', action='store_true',
                        help='discover the target Quote Mint from the selected pool')
    parser.add_argument('--amm-v4-simulate-only', '--amm-v4-readonly',
                        dest='amm_v4_simulate_only', action='store_true',
                        help='verify the public RAY/USDC AMM v4 two-hop decision and simulate-only trade')
    parser.add_argument('--cpmm-simulate-only', action='store_true',
                        help='verify a public USDC-quoted CPMM target via a CPMM SOL bridge')
    parser.add_argument('--single-pool-simulate-only', action='store_true',
                        help='verify a public WSOL-quoted PumpSwap buy through the SDK trade API')
    parser.add_argument('--single-pool-sell-simulate-only', action='store_true',
                        help='verify an owned WSOL sell through the chain-verified WSOL/USDC CPMM pool')
    parser.add_argument('--bot-shaped-single-pool', action='store_true',
                        help='send the Bot manual SDK request fields in the single-pool check')
    parser.add_argument('--bot-shaped-bridge', action='store_true',
                        help='send the Bot manual SDK request fields in the two-hop check')
    parser.add_argument('--check-panel-resolution', action='store_true',
                        help='also require mint-only panel pool resolution to find the fixture')
    parser.add_argument('--panel-resolution-only', action='store_true',
                        help='stop after checking candidate panel pool resolution')
    args = parser.parse_args()
    if sum((args.amm_v4_simulate_only, args.cpmm_simulate_only,
            args.single_pool_simulate_only, args.single_pool_sell_simulate_only)) > 1:
        parser.error('select one fixture')
    if args.bot_shaped_single_pool and not (args.single_pool_simulate_only or
                                            args.single_pool_sell_simulate_only):
        parser.error('--bot-shaped-single-pool requires a single-pool simulation mode')
    if args.bot_shaped_bridge and (args.single_pool_simulate_only or
                                   args.single_pool_sell_simulate_only):
        parser.error('--bot-shaped-bridge requires a two-hop simulation mode')
    if args.check_panel_resolution and not args.bot_shaped_bridge:
        parser.error('--check-panel-resolution requires --bot-shaped-bridge')
    if args.panel_resolution_only and not args.check_panel_resolution:
        parser.error('--panel-resolution-only requires --check-panel-resolution')
    binary = args.binary.resolve(strict=True)
    candidate_dir = args.candidate_dir.resolve(strict=True)
    wallet_db = args.wallet_db.resolve(strict=True)

    with tempfile.TemporaryDirectory(prefix='funded-sdk-preview-') as name:
        root = Path(name)
        copied_wallet_db = root / 'wallets.sqlite'
        copy_database(wallet_db, copied_wallet_db)
        with sqlite3.connect(copied_wallet_db) as connection:
            wallet = connection.execute(
                'SELECT pubkey FROM wallets WHERE wallet_id=? AND enabled=1',
                (args.wallet_id,),
            ).fetchone()
        if wallet is None:
            raise ValueError('selected enabled wallet is unavailable')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = 'isolated-funded-sdk-preview-token'
        trade_db = root / 'trades.sqlite'
        env = dict(os.environ,
                   TRADE_HTTP_ADDR=f'127.0.0.1:{port}',
                   TRADE_DATABASE_URL='sqlite://' + str(trade_db),
                   WALLET_DATABASE_URL='sqlite://' + str(copied_wallet_db),
                   CONVERSION_DATABASE_URL='sqlite://' + str(root / 'conversions.sqlite'),
                   TRADE_PERSISTENCE_MODE='durable',
                   WALLET_STORAGE_MODE='database',
                   TRADE_SERVICE_SIMULATE_ONLY='true',
                   ENABLE_SIMULATE='true',
                   CONVERSION_LIVE_ENABLED='false',
                   CONVERSION_QUOTES_ENABLED='false',
                   ENABLE_PUMPAPI_EXECUTOR='false',
                   LOCAL_TRADE_API_TOKEN=token,
                   RUST_LOG='error')
        with (root / 'service.log').open('w') as log:
            proc = subprocess.Popen([str(binary)], cwd=candidate_dir, env=env,
                                    stdout=log, stderr=subprocess.STDOUT)
            try:
                for _ in range(300):
                    if proc.poll() is not None:
                        raise RuntimeError('isolated service exited during startup')
                    try:
                        if http(base, None, '/health')[0] == 200:
                            break
                    except (OSError, ValueError):
                        pass
                    time.sleep(0.2)
                else:
                    raise RuntimeError('isolated service did not become healthy')

                if args.single_pool_simulate_only or args.single_pool_sell_simulate_only:
                    trade = {
                        'wallet_id': args.wallet_id, 'side': 'buy', 'pool': 'pump-amm',
                        'mint': 'pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn',
                        'pool_id': '539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR',
                        'quote_mint': 'So11111111111111111111111111111111111111112',
                        'amount_lamports': 1_000_000, 'slippage_bps': 100,
                        'execution_method': 'sdk', 'simulate': True,
                        'idempotency_key': 'isolated-funded-sdk-single-pool',
                    }
                    if args.single_pool_sell_simulate_only:
                        trade.update({
                            'side': 'sell', 'pool': 'raydium-cpmm',
                            'mint': 'So11111111111111111111111111111111111111112',
                            'pool_id': 'fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2',
                            'quote_mint': 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                            'sell_percent': 10,
                            'idempotency_key': 'isolated-funded-sdk-single-pool-sell',
                        })
                        trade.pop('amount_lamports')
                    if args.bot_shaped_single_pool:
                        if trade['side'] == 'buy':
                            trade.pop('amount_lamports')
                            trade['amount_sol'] = 0.001
                        trade.pop('slippage_bps')
                        trade.update({
                            'slippage_percent': 1.0,
                            'tip_sol': 0.0001, 'use_mev': False,
                            'mev_provider': None, 'amount_quote_units': None,
                            'settlement_asset': None,
                            'sol_hop_pool': None, 'sol_hop_pool_id': None,
                            'source': 'copy-trading-manual', 'source_signature': None,
                        })
                        if trade['side'] == 'buy':
                            trade['sell_percent'] = None
                    if args.omit_quote:
                        trade.pop('quote_mint')
                    status, decision = http(base, token, '/v1/routes/decide', trade)
                    assert status == 200 and decision['status'] == 'sdk_verified', (status, decision)
                    assert decision['wallet_pubkey'] == wallet[0], decision
                    assert decision['sdk_capability']['service_adapter'] == 'verified_direct_sdk', decision
                    simulated_trade = dict(trade, sdk_decision_id=decision['decision_id'])
                    status, accepted = http(base, token, '/v1/trades', simulated_trade)
                    assert status in (200, 202) and accepted['request_id'], (status, accepted)
                    request_id = accepted['request_id']
                    for _ in range(150):
                        if accepted['status'] not in ('accepted', 'warming_params', 'building_tx', 'submitting'):
                            break
                        time.sleep(0.2)
                        status, accepted = http(base, token, f'/v1/trades/{request_id}')
                        assert status == 200, (status, accepted)
                    assert accepted['status'] == 'simulated' and accepted['signature'] is None, accepted
                    assert accepted['simulation']['ok'] is True, accepted
                    status, duplicate = http(base, token, '/v1/trades', simulated_trade)
                    assert status == 200 and duplicate['duplicate'] is True, (status, duplicate)
                    assert duplicate['request_id'] == request_id and duplicate['signature'] is None, duplicate
                    live_status, _ = http(base, token, '/v1/trades', dict(
                        simulated_trade, simulate=False,
                        idempotency_key='isolated-funded-sdk-single-pool-live-reject'))
                    assert live_status in (400, 403), live_status
                    with sqlite3.connect(trade_db) as connection:
                        counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                                  for table in ('trades', 'aggregator_quotes', 'aggregator_submissions')}
                    assert counts == {'trades': 1, 'aggregator_quotes': 0, 'aggregator_submissions': 0}, counts
                    proc.terminate()
                    proc.wait(timeout=5)
                    proc = subprocess.Popen([str(binary)], cwd=candidate_dir, env=env,
                                            stdout=log, stderr=subprocess.STDOUT)
                    for _ in range(300):
                        if proc.poll() is not None:
                            raise RuntimeError('isolated single-pool service exited during restart')
                        try:
                            if http(base, None, '/health')[0] == 200:
                                break
                        except (OSError, ValueError):
                            pass
                        time.sleep(0.2)
                    else:
                        raise RuntimeError('isolated single-pool service did not restart')
                    status, duplicate = http(base, token, '/v1/trades', simulated_trade)
                    assert status == 200 and duplicate['duplicate'] is True, (status, duplicate)
                    assert duplicate['request_id'] == request_id and duplicate['signature'] is None, duplicate
                    status, readback = http(base, token, f'/v1/trades/{request_id}')
                    assert status == 200 and readback['status'] == 'simulated', (status, readback)
                    assert readback['signature'] is None and readback['simulation']['ok'] is True, readback
                    new_trade = dict(simulated_trade, idempotency_key='isolated-single-pool-stale-decision')
                    stale_status, _ = http(base, token, '/v1/trades', new_trade)
                    assert stale_status == 400, stale_status
                    with sqlite3.connect(trade_db) as connection:
                        restarted_counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                                            for table in counts}
                    assert restarted_counts == counts, restarted_counts
                    print(json.dumps({
                        'passed': True, 'route': ('cpmm_wsol_sell_simulate_only'
                                                  if args.single_pool_sell_simulate_only else
                                                  'pumpswap_single_pool_simulate_only'),
                        'bot_shaped': args.bot_shaped_single_pool,
                        'decision': decision['status'], 'trade_status': accepted['status'],
                        'simulation_ok': accepted['simulation']['ok'],
                        'signature_exposed': False, 'idempotent_after_restart': True,
                        'live_rejected': True, 'records': counts,
                    }, sort_keys=True))
                    return

                if args.amm_v4_simulate_only or args.cpmm_simulate_only:
                    route_kind = 'cpmm' if args.cpmm_simulate_only else 'amm-v4'
                    trade = {
                        'wallet_id': args.wallet_id, 'side': 'buy',
                        'pool': 'raydium-cpmm' if args.cpmm_simulate_only else 'raydium-amm-v4',
                        'mint': ('3ZLekZYq2qkZiSpnSvabjit34tUkjSwD1JFuW9as9wBG'
                                 if args.cpmm_simulate_only else '4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R'),
                        'pool_id': ('gyYigqG8vdEMkDNTTVL6at2msbHdTDWyb6bccyR238U'
                                    if args.cpmm_simulate_only else '6UmmUiYoBjSrhakAobJw8BvkmJtDVxaeBtbt7rxWo1mg'),
                        'quote_mint': 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                        'amount_sol': 0.001, 'slippage_bps': 100,
                        'execution_method': 'sdk', 'simulate': True,
                        'idempotency_key': f'isolated-funded-{route_kind}-readonly',
                    }
                    if args.omit_quote:
                        trade.pop('quote_mint')
                    if args.bot_shaped_bridge:
                        trade.pop('slippage_bps')
                        trade.update({
                            'slippage_percent': 1.0,
                            'amount_quote_units': None, 'settlement_asset': None,
                            'sol_hop_pool': None, 'sol_hop_pool_id': None,
                            'sell_percent': None, 'source': 'copy-trading-manual',
                            'source_signature': None,
                        })
                        if args.check_panel_resolution:
                            panel_resolution = verify_panel_resolution(
                                base, token, trade,
                                'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v')
                            if args.panel_resolution_only:
                                print(json.dumps({'passed': True, 'panel_resolution': panel_resolution,
                                                  'trades_submitted': 0}, sort_keys=True))
                                return
                    last_result = None
                    for attempt in range(5):
                        status, decision = http(base, token, '/v1/routes/decide', trade)
                        if status != 200 or decision.get('reason') != 'bridge_verified_sdk_capability_unresolved':
                            last_result = (f'{route_kind} bridge observation unavailable', status, decision.get('reason'))
                            time.sleep(1)
                            continue
                        observation = decision['observed_bridge']
                        assert decision['decision_id'] is None and observation['wallet_pubkey'] == wallet[0]
                        status, preview = http(base, token, '/v1/routes/observations/sdk-preview', {
                            'observation_id': observation['observation_id'],
                            'trade': trade, 'simulate_transaction': True,
                        })
                        if status == 200 and preview.get('preview_id') and preview['preview'].get('simulation'):
                            break
                        last_result = (f'{route_kind} unsigned simulation unavailable', status, preview.get('error'))
                        if attempt < 4:
                            time.sleep(1)
                    else:
                        raise AssertionError(last_result)
                    simulation = preview['preview']['simulation']
                    assert preview['preview_id'].startswith('observe:sdk-preview:')
                    assert preview['executable'] is False, preview
                    assert simulation['attempted'] is True and simulation['rpc_accepted'] is True, simulation
                    assert simulation['ok'] is True and simulation['minimum_output_verified'] is True, simulation
                    assert simulation['target_token_delta_units'] >= preview['preview']['target_min_output_units'], simulation
                    assert simulation['payer_debit_lamports'] >= 1_000_000, simulation
                    status, sdk_decision = http(base, token, '/v1/routes/decide',
                                                dict(trade, sdk_preview_id=preview['preview_id']))
                    assert status == 200 and sdk_decision['status'] == 'sdk_verified', (status, sdk_decision)
                    assert sdk_decision['decision_id'].startswith('sdk-decision:bridge:'), sdk_decision
                    simulation_trade = dict(trade, sdk_decision_id=sdk_decision['decision_id'])
                    status, accepted = http(base, token, '/v1/trades', simulation_trade)
                    assert status in (200, 202), (status, accepted)
                    request_id = accepted['request_id']
                    for _ in range(150):
                        if accepted['status'] not in ('accepted', 'warming_params', 'building_tx', 'submitting'):
                            break
                        time.sleep(0.2)
                        status, accepted = http(base, token, f'/v1/trades/{request_id}')
                        assert status == 200, (status, accepted)
                    assert accepted['status'] in ('simulated', 'failed'), accepted
                    assert accepted['signature'] is None, accepted
                    assert accepted['simulation']['source'] == 'sdk-11:stonkfun-quote-route', accepted
                    if accepted['status'] == 'failed':
                        assert accepted['error'] == 'SDK two-hop current minimum output fell below bound preview', accepted
                    else:
                        assert accepted['simulation']['ok'] is True, accepted
                    status, duplicate = http(base, token, '/v1/trades', simulation_trade)
                    assert status == 200 and duplicate['duplicate'] is True, (status, duplicate)
                    assert duplicate['request_id'] == request_id and duplicate['signature'] is None, duplicate
                    tampered = dict(simulation_trade, amount_sol=0.002,
                                    idempotency_key=f'tampered-{route_kind}-preview')
                    tampered_status, _ = http(base, token, '/v1/trades', tampered)
                    assert tampered_status == 400, tampered_status
                    wrong_pool = dict(simulation_trade, pool='pump-amm',
                                      idempotency_key=f'wrong-pool-{route_kind}-preview')
                    wrong_pool_status, _ = http(base, token, '/v1/trades', wrong_pool)
                    assert wrong_pool_status == 400, wrong_pool_status
                    live_attempt = dict(simulation_trade, simulate=False,
                                        idempotency_key=f'live-{route_kind}-preview')
                    live_attempt_status, _ = http(base, token, '/v1/trades', live_attempt)
                    assert live_attempt_status == 400, live_attempt_status
                    with sqlite3.connect(trade_db) as connection:
                        counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                                  for table in ('trades', 'aggregator_quotes', 'aggregator_submissions')}
                    assert counts == {'trades': 1, 'aggregator_quotes': 0, 'aggregator_submissions': 0}, counts
                    proc.terminate()
                    proc.wait(timeout=5)
                    proc = subprocess.Popen([str(binary)], cwd=candidate_dir, env=env,
                                            stdout=log, stderr=subprocess.STDOUT)
                    for _ in range(300):
                        if proc.poll() is not None:
                            raise RuntimeError(f'isolated {route_kind} service exited during restart')
                        try:
                            if http(base, None, '/health')[0] == 200:
                                break
                        except (OSError, ValueError):
                            pass
                        time.sleep(0.2)
                    else:
                        raise RuntimeError(f'isolated {route_kind} service did not restart')
                    status, restarted_duplicate = http(base, token, '/v1/trades', simulation_trade)
                    assert status == 200 and restarted_duplicate['duplicate'] is True, (status, restarted_duplicate)
                    assert restarted_duplicate['request_id'] == request_id, restarted_duplicate
                    stale_new_trade = dict(simulation_trade, idempotency_key=f'stale-{route_kind}-after-restart')
                    stale_status, _ = http(base, token, '/v1/trades', stale_new_trade)
                    assert stale_status == 400, stale_status
                    with sqlite3.connect(trade_db) as connection:
                        restarted_counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                                            for table in counts}
                    assert restarted_counts == counts, restarted_counts
                    print(json.dumps({
                        'passed': True, 'route': f'cpmm_to_{route_kind.replace("-", "_")}_simulate_only',
                        'simulation_ok': simulation['ok'],
                        'minimum_output_verified': simulation['minimum_output_verified'],
                        'target_min_output_units': preview['preview']['target_min_output_units'],
                        'target_token_delta_units': simulation['target_token_delta_units'],
                        'payer_debit_lamports': simulation['payer_debit_lamports'],
                        'units_consumed': simulation['units_consumed'],
                        'transaction_version': simulation['transaction_version'],
                        'transaction_bytes': simulation['transaction_bytes'],
                        'preview_id_issued': True, 'trade_status': accepted['status'],
                        'idempotent_after_restart': True, 'records': counts,
                        'panel_resolution': panel_resolution if args.check_panel_resolution else None,
                    }, sort_keys=True))
                    return

                trade = {
                    'wallet_id': args.wallet_id,
                    'side': 'buy',
                    'pool': 'pump-amm',
                    'mint': 'nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump',
                    'pool_id': 'G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b',
                    'quote_mint': 'TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo',
                    'amount_sol': 0.001,
                    'slippage_bps': 100,
                    'execution_method': 'sdk',
                    'simulate': True,
                    'idempotency_key': 'isolated-funded-sdk-preview',
                }
                if args.omit_quote:
                    trade.pop('quote_mint')
                if args.bot_shaped_bridge:
                    trade.pop('slippage_bps')
                    trade.update({
                        'slippage_percent': 1.0,
                        'amount_quote_units': None, 'settlement_asset': None,
                        'sol_hop_pool': None, 'sol_hop_pool_id': None,
                        'sell_percent': None, 'source': 'copy-trading-manual',
                        'source_signature': None,
                    })
                    if args.check_panel_resolution:
                        panel_resolution = verify_panel_resolution(
                            base, token, trade,
                            'TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo')
                        if args.panel_resolution_only:
                            print(json.dumps({'passed': True, 'panel_resolution': panel_resolution,
                                              'trades_submitted': 0}, sort_keys=True))
                            return
                def get_funded_preview():
                    last_result = None
                    for attempt in range(5):
                        status, decision = http(base, token, '/v1/routes/decide', trade)
                        if status != 200 or decision.get('reason') != 'bridge_verified_sdk_capability_unresolved':
                            last_result = ('route decision unavailable', status, decision.get('reason'))
                            time.sleep(1)
                            continue
                        observation = decision['observed_bridge']
                        assert decision['decision_id'] is None and observation['wallet_pubkey'] == wallet[0]
                        status, preview = http(base, token, '/v1/routes/observations/sdk-preview', {
                            'observation_id': observation['observation_id'],
                            'trade': trade,
                            'simulate_transaction': True,
                        })
                        if status == 200 and preview.get('preview_id'):
                            return decision, observation, preview
                        last_result = ('SDK preview unavailable', status, preview.get('error'))
                        if attempt < 4:
                            time.sleep(1)
                    raise AssertionError(last_result)

                decision, observation, preview = get_funded_preview()
                assert preview['executable'] is False and preview['simulate_only'] is True
                simulation = preview['preview']['simulation']
                assert simulation['attempted'] is True and simulation['rpc_accepted'] is True
                assert simulation['ok'] is True and simulation['error'] is None, simulation
                assert simulation['minimum_output_verified'] is True, simulation
                assert simulation['target_token_delta_units'] >= preview['preview']['target_min_output_units'], simulation
                assert simulation['target_token_post_units'] >= simulation['target_token_delta_units'], simulation
                assert simulation['payer_debit_lamports'] >= 1000000, simulation
                preview_id = preview.get('preview_id')
                assert isinstance(preview_id, str) and preview_id.startswith('observe:sdk-preview:'), preview
                readback_request = {'preview_id': preview_id, 'trade': trade}
                readback_status, readback = http(base, token,
                                                 '/v1/routes/observations/sdk-preview/readback',
                                                 readback_request)
                assert readback_status == 200 and readback['status'] == 'sdk_preview_verified_read_only', readback
                assert readback['executable'] is False and readback['simulate_only'] is True, readback
                assert readback['target_min_output_units'] == preview['preview']['target_min_output_units'], readback
                assert readback['target_token_delta_units'] == simulation['target_token_delta_units'], readback
                altered = dict(trade, amount_sol=0.002)
                altered_status, _ = http(base, token,
                                         '/v1/routes/observations/sdk-preview/readback',
                                         {'preview_id': preview_id, 'trade': altered})
                assert altered_status == 400, altered_status
                rejected_trade = dict(trade, sdk_decision_id=preview_id)
                rejected_status, _ = http(base, token, '/v1/trades', rejected_trade)
                assert rejected_status == 400, rejected_status
                with sqlite3.connect(trade_db) as connection:
                    counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                              for table in ('trades', 'aggregator_quotes', 'aggregator_submissions')}
                assert all(count == 0 for count in counts.values()), counts

                # Local readback checks do not require another RPC preview.
                # Convert this successful five-second credential immediately.
                bridge_decision_status, bridge_decision = http(
                    base, token, '/v1/routes/decide',
                    dict(trade, sdk_preview_id=preview_id))
                assert bridge_decision_status == 200 and bridge_decision['status'] == 'sdk_verified', (
                    bridge_decision_status, bridge_decision)
                assert bridge_decision['decision_id'].startswith('sdk-decision:bridge:'), bridge_decision
                assert bridge_decision['route_id'] == preview['observation_id'], bridge_decision
                assert readback['observation_id'] == preview['observation_id'], readback
                assert bridge_decision['sdk_capability'] == {
                    'package_builder_family': 'PumpSwapQuoteRoute',
                    'service_adapter': 'simulate_only_bridge_decision',
                    'route_evidence': 'unsigned_simulation_verified',
                    'sdk_unsupported_proven': False,
                    'jupiter_candidate_allowed': False,
                }, bridge_decision
                assert bridge_decision['expires_at_ms'] == readback['expires_at_ms'], bridge_decision
                if preview['observation_id'] == observation['observation_id']:
                    assert bridge_decision['expires_at_ms'] <= observation['expires_at_ms'], bridge_decision
                simulation_trade = dict(trade, sdk_decision_id=bridge_decision['decision_id'])
                status, accepted = http(base, token, '/v1/trades', simulation_trade)
                assert status in (200, 202), (status, accepted)
                request_id = accepted['request_id']
                for _ in range(150):
                    if accepted['status'] not in ('accepted', 'warming_params', 'building_tx', 'submitting'):
                        break
                    time.sleep(0.2)
                    status, accepted = http(base, token, f'/v1/trades/{request_id}')
                    assert status == 200, (status, accepted)
                if args.omit_quote and accepted['status'] == 'failed':
                    # A moving pool may fall below the first preview's bound
                    # during the second simulation. That must fail the same
                    # SDK order without exposing a signature or switching backend.
                    assert accepted['error'] == 'SDK two-hop current minimum output fell below bound preview', accepted
                    assert accepted['simulation']['ok'] is False, accepted
                else:
                    assert accepted['status'] == 'simulated', accepted
                    assert accepted['simulation']['ok'] is True, accepted
                assert accepted['signature'] is None, accepted
                assert accepted['simulation']['source'] == 'sdk-11:pumpswap-quote-route', accepted
                status, duplicate = http(base, token, '/v1/trades', simulation_trade)
                assert status == 200 and duplicate['duplicate'] is True, duplicate
                assert duplicate['request_id'] == request_id and duplicate['signature'] is None, duplicate
                invalid_trade = dict(simulation_trade, simulate=False)
                invalid_status, _ = http(base, token, '/v1/trades', invalid_trade)
                assert invalid_status == 400, invalid_status
                altered_trade = dict(simulation_trade, amount_sol=0.002,
                                     idempotency_key='tampered-sdk-preview')
                altered_status, _ = http(base, token, '/v1/trades', altered_trade)
                assert altered_status == 400, altered_status
                changed_source = dict(simulation_trade, source='changed-copy-source')
                changed_source_status, _ = http(base, token, '/v1/trades', changed_source)
                assert changed_source_status == 409, changed_source_status
                with sqlite3.connect(trade_db) as connection:
                    counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                              for table in counts}
                assert counts == {'trades': 1, 'aggregator_quotes': 0, 'aggregator_submissions': 0}, counts
                proc.terminate()
                proc.wait(timeout=5)
                proc = subprocess.Popen([str(binary)], cwd=candidate_dir, env=env,
                                        stdout=log, stderr=subprocess.STDOUT)
                for _ in range(300):
                    if proc.poll() is not None:
                        raise RuntimeError('isolated service exited during restart')
                    try:
                        if http(base, None, '/health')[0] == 200:
                            break
                    except (OSError, ValueError):
                        pass
                    time.sleep(0.2)
                else:
                    raise RuntimeError('isolated service did not restart')
                old_status, _ = http(base, token,
                                     '/v1/routes/observations/sdk-preview/readback',
                                     readback_request)
                assert old_status == 400, old_status
                restarted_trade_status, restarted_duplicate = http(base, token, '/v1/trades', simulation_trade)
                assert restarted_trade_status == 200 and restarted_duplicate['duplicate'] is True, restarted_duplicate
                assert restarted_duplicate['request_id'] == request_id, restarted_duplicate
                stale_new_trade = dict(simulation_trade, idempotency_key='stale-after-restart')
                stale_status, _ = http(base, token, '/v1/trades', stale_new_trade)
                assert stale_status == 400, stale_status
                with sqlite3.connect(trade_db) as connection:
                    restarted_counts = {table: connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]
                                        for table in counts}
                assert restarted_counts == counts, restarted_counts
                print(json.dumps({
                    'passed': True,
                    'quote_mint_discovered': args.omit_quote,
                    'route_decision': decision['reason'],
                    'bridge_sdk_decision': bridge_decision['status'],
                    'bridge_observation_refreshed': preview['observation_id'] != observation['observation_id'],
                    'preview_id_issued': True,
                    'preview_id_as_trade_rejected': True,
                    'preview_readback_tamper_rejected': True,
                    'preview_restart_rejected': True,
                    'preview_trade_status': accepted['status'],
                    'preview_trade_idempotent': True,
                    'preview_new_trade_after_restart_rejected': True,
                    'preview_exact_retry_after_restart_idempotent': True,
                    'simulation_ok': simulation['ok'],
                    'simulation_error': simulation['error'],
                    'simulation_slot': simulation['slot'],
                    'units_consumed': simulation['units_consumed'],
                    'payer_debit_lamports': simulation['payer_debit_lamports'],
                    'target_token_post_units': simulation['target_token_post_units'],
                    'target_token_delta_units': simulation['target_token_delta_units'],
                    'minimum_output_verified': simulation['minimum_output_verified'],
                    'target_min_output_units': preview['preview']['target_min_output_units'],
                    'transaction_version': simulation['transaction_version'],
                    'transaction_bytes': simulation['transaction_bytes'],
                    'panel_resolution': panel_resolution if args.check_panel_resolution else None,
                    'records': counts,
                }, sort_keys=True))
            finally:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=5)


if __name__ == '__main__':
    main()
