#!/usr/bin/env python3
"""Exercise route decision HTTP boundaries on a disposable simulate-only service."""

import argparse
import json
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def b58(data):
    n = int.from_bytes(data, 'big')
    digits = []
    while n:
        n, rem = divmod(n, 58)
        digits.append(ALPHABET[rem])
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + ''.join(reversed(digits))


def request(base, token, path, body=None):
    data = None if body is None else json.dumps(body).encode()
    headers = {'content-type': 'application/json'}
    if token is not None:
        headers['authorization'] = 'Bearer ' + token
    req = urllib.request.Request(base + path, data=data, headers=headers,
                                 method='POST' if body is not None else 'GET')
    try:
        with urllib.request.urlopen(req, timeout=15) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


def count(db, table):
    with sqlite3.connect(db) as connection:
        return connection.execute(f'SELECT COUNT(*) FROM {table}').fetchone()[0]


def rpc_url_from_file(path):
    for line in path.read_text().splitlines():
        if line.startswith('RPC_URL='):
            value = line.split('=', 1)[1].strip().strip('"\'')
            if value:
                return value
    raise ValueError('RPC_URL is missing from the supplied env file')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True,
                        help='isolated candidate release binary to verify')
    parser.add_argument('--positive-pumpswap-fixture', action='store_true',
                        help='verify the public PumpSwap example pool over a real read-only RPC')
    parser.add_argument('--positive-amm-v4-fixture', action='store_true',
                        help='verify read-only quotes for a public Raydium AMM v4 pool')
    parser.add_argument('--positive-amm-v4-bridge-fixture', action='store_true',
                        help='verify a public RAY/USDC AMM v4 target with a CPMM WSOL/USDC bridge')
    parser.add_argument('--probe-pump-usdc-mint',
                        help='read-only probe of a public Pump.fun USDC curve mint')
    parser.add_argument('--simulate-valid-decision', action='store_true',
                        help='also admit one simulate-only SDK trade in the disposable file-wallet service')
    parser.add_argument('--rpc-url-file', type=Path,
                        help='env file containing RPC_URL; required for the positive fixture')
    args = parser.parse_args()
    if bool(args.positive_pumpswap_fixture or args.positive_amm_v4_fixture
            or args.positive_amm_v4_bridge_fixture or args.probe_pump_usdc_mint) != bool(args.rpc_url_file):
        parser.error('a positive fixture and --rpc-url-file must be used together')
    if args.simulate_valid_decision and not args.positive_pumpswap_fixture:
        parser.error('--simulate-valid-decision requires --positive-pumpswap-fixture')
    source_binary = args.binary.resolve(strict=True)
    rpc_url = rpc_url_from_file(args.rpc_url_file) if args.rpc_url_file else 'http://127.0.0.1:9'
    with tempfile.TemporaryDirectory(prefix='route-decision-http-') as root_name:
        root = Path(root_name)
        binary = root / 'pump-trade-service'
        shutil.copy2(source_binary, binary)
        signer = Ed25519PrivateKey.generate()
        seed = signer.private_bytes(serialization.Encoding.Raw,
                                    serialization.PrivateFormat.Raw,
                                    serialization.NoEncryption())
        pubkey = signer.public_key().public_bytes(serialization.Encoding.Raw,
                                                  serialization.PublicFormat.Raw)
        wallet_file = root / 'wallets.json'
        wallet_file.write_text(json.dumps({'wallets': [
            {'wallet_id': 'isolated', 'private_key_base58': b58(seed + pubkey)}]}))
        wallet_file.chmod(0o600)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        base = f'http://127.0.0.1:{port}'
        token = 'isolated-route-decision-token'
        db = root / 'trades.sqlite'
        env = {
            'PATH': '/usr/bin:/bin',
            'RPC_URL': rpc_url,
            'LOCAL_TRADE_API_TOKEN': token,
            'DEFAULT_WALLET_ID': 'isolated',
            'WALLET_STORAGE_MODE': 'file',
            'WALLETS_CONFIG': str(wallet_file),
            'TRADE_HTTP_ADDR': f'127.0.0.1:{port}',
            'TRADE_PERSISTENCE_MODE': 'durable',
            'TRADE_DATABASE_URL': 'sqlite://' + str(db),
            'TRADE_SERVICE_SIMULATE_ONLY': 'true',
            'ENABLE_SIMULATE': 'true',
            'CONVERSION_QUOTES_ENABLED': 'false',
            'CONVERSION_LIVE_ENABLED': 'false',
            'ENABLE_MEV_PROTECTION': 'false',
            'REQUIRE_MEV_SUBMIT': 'false',
            'ENABLE_PUMPAPI_EXECUTOR': 'false',
            'RUST_LOG': 'warn' if args.rpc_url_file else 'error',
        }
        log = (root / 'service.log').open('w')
        proc = subprocess.Popen([str(binary)], cwd=root, env=env,
                                stdout=log, stderr=subprocess.STDOUT)
        try:
            for _ in range(80):
                if proc.poll() is not None:
                    raise RuntimeError(f'service exited {proc.returncode}; inspect isolated service log locally')
                try:
                    status, _ = request(base, None, '/health')
                    if status == 200:
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(0.25)
            else:
                raise RuntimeError('service did not become healthy')

            trade = {
                'wallet_id': 'isolated', 'side': 'buy', 'pool': 'raydium-cpmm',
                'mint': 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                'pool_id': '11111111111111111111111111111111',
                'quote_mint': 'So11111111111111111111111111111111111111112',
                'amount_lamports': 1000000, 'slippage_bps': 100,
                'execution_method': 'sdk', 'simulate': True,
                'idempotency_key': 'decision-http-1',
            }
            status, _ = request(base, None, '/v1/routes/decide', trade)
            assert status == 401, f'unauthorized status={status}'
            status, capabilities = request(base, token, '/v1/capabilities')
            assert status == 200 and capabilities['jupiter_trade_requires_sdk_unsupported'] is True, (
                status, capabilities)
            pump_amm_capability = next(item for item in capabilities['protocols']
                                       if item['protocol_id'] == 'pump-amm')
            assert pump_amm_capability['sdk_package_route_builder'] == 'PumpSwapQuoteRoute', pump_amm_capability
            assert pump_amm_capability['service_sol_route_adapter'] == 'cpmm_pumpswap_simulate_only', pump_amm_capability
            assert pump_amm_capability['supports_sol_route'] is False, pump_amm_capability
            for protocol_id in ('raydium-clmm', 'orca-whirlpool', 'meteora-dlmm'):
                venue = next(item for item in capabilities['protocols']
                             if item['protocol_id'] == protocol_id)
                assert venue['sdk_package_route_builder'] == 'StonkFunQuoteRoute', venue
                assert venue['service_sol_route_adapter'] == 'not_integrated', venue
                assert venue['supports_sol_route'] is False, venue
            cpmm_capability = next(item for item in capabilities['protocols']
                                   if item['protocol_id'] == 'raydium-cpmm')
            assert cpmm_capability['sdk_package_route_builder'] == 'StonkFunQuoteRoute', cpmm_capability
            assert cpmm_capability['service_sol_route_adapter'] == 'cpmm_cpmm_simulate_only', cpmm_capability
            assert cpmm_capability['supports_sol_route'] is False, cpmm_capability
            amm_v4_capability = next(item for item in capabilities['protocols']
                                     if item['protocol_id'] == 'raydium-amm-v4')
            assert amm_v4_capability['sdk_package_route_builder'] == 'StonkFunQuoteRoute', amm_v4_capability
            assert amm_v4_capability['service_sol_route_adapter'] == 'cpmm_amm_v4_simulate_only', amm_v4_capability
            assert amm_v4_capability['supports_sol_route'] is False, amm_v4_capability
            no_wallet = dict(trade, wallet_id='missing')
            status, body = request(base, token, '/v1/routes/decide', no_wallet)
            assert status == 200 and body['status'] == 'policy_blocked' and body['reason'] == 'wallet_unavailable', (status, body)
            assert body.get('wallet_pubkey') is None, body
            aggregator = dict(trade, execution_method='aggregator')
            status, body = request(base, token, '/v1/routes/decide', aggregator)
            assert status == 200 and body['status'] == 'policy_blocked' and body['reason'] == 'invalid_sdk_trade_request', (status, body)
            assert body.get('wallet_pubkey') == b58(pubkey), body
            aggregator_trade = dict(trade,
                                    quote_mint='EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                                    amount_sol=0.001,
                                    execution_method='aggregator',
                                    idempotency_key='explicit-aggregator-bypass')
            aggregator_trade.pop('amount_lamports')
            status, body = request(base, token, '/v1/trades', aggregator_trade)
            assert status == 409 and 'sdk_unsupported' in body['error'], (status, body)
            assert count(db, 'trades') == 0, 'client-selected aggregator consumed a trade key'
            pump_usdc = dict(trade, pool='pump',
                             quote_mint='EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                             amount_sol=0.001,
                             idempotency_key='pump-usdc-readonly-credential')
            pump_usdc.pop('pool_id')
            pump_usdc.pop('amount_lamports')
            for credential in ({'sdk_preview_id': 'observe:sdk-preview:fake'},
                               {'sdk_decision_id': 'sdk-decision:bridge:fake'}):
                attempted = dict(pump_usdc, **credential)
                status, body = request(base, token, '/v1/routes/decide', attempted)
                if 'sdk_preview_id' in credential:
                    assert status == 200 and body['status'] == 'policy_blocked', (status, body)
                    assert body['reason'] == 'pumpfun_token_quote_preview_is_read_only', body
                status, _ = request(base, token, '/v1/trades', attempted)
                assert status == 400, f'Pump token-quote credential entered trade admission: {status}'
                assert count(db, 'trades') == 0, 'Pump token-quote credential wrote a trade'
            pump_usdc_probe = None
            if args.probe_pump_usdc_mint:
                live_pump = dict(pump_usdc, mint=args.probe_pump_usdc_mint,
                                 idempotency_key='pump-usdc-readonly-probe')
                status, decision = request(base, token, '/v1/routes/decide', live_pump)
                assert status == 200 and decision['decision_id'] is None, (status, decision)
                assert decision['status'] == 'unresolved_retryable', decision
                if decision.get('sdk_capability'):
                    assert decision['sdk_capability']['jupiter_candidate_allowed'] is False, decision
                pump_usdc_probe = {'decision_status': decision['status'],
                                   'decision_reason': decision['reason'],
                                   'bridge_observed': decision.get('observed_bridge') is not None}
                observation = decision.get('observed_bridge')
                if observation:
                    status, preview = request(base, token, '/v1/routes/observations/sdk-preview', {
                        'observation_id': observation['observation_id'],
                        'trade': live_pump, 'simulate_transaction': False,
                    })
                    pump_usdc_probe['preview_http_status'] = status
                    if status == 200:
                        assert preview['preview_id'] is None and preview['executable'] is False, preview
                        pump_usdc_probe['builder'] = preview['preview']['builder']
                        pump_usdc_probe['instruction_count'] = preview['preview']['instruction_count']
                assert count(db, 'trades') == 0 and count(db, 'aggregator_quotes') == 0
                assert count(db, 'aggregator_submissions') == 0
            status, body = request(base, token, '/v1/routes/decide', trade)
            assert status == 200 and body['status'] == 'unresolved_retryable' and body['reason'] == 'route_verification_unavailable', (status, body)
            assert body['decision_id'] is None and body['simulate_only'] is True, body
            positive_status = None
            mint_only_pumpswap_status = None
            restart_status = None
            bridge_status = None
            old_observation_id = None
            sdk_preview_summary = None
            amm_v4_quote_status = None
            amm_v4_bridge_status = None
            if args.positive_amm_v4_fixture:
                amm_hint = {
                    'protocol_id': 'raydium-amm-v4',
                    'pool_id': '58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2',
                    'mint': 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                    'quote_mint': 'So11111111111111111111111111111111111111112',
                }
                for _ in range(3):
                    status, amm_route = request(base, token, '/v1/pools/resolve-candidates', amm_hint)
                    assert status == 200 and amm_route['chain_verified'] is True, (status, amm_route)
                    assert amm_route['pool_id'] == amm_hint['pool_id'], amm_route
                    buy_request = {
                        'route': amm_route, 'wallet_id': 'isolated',
                        'input_amount_units': 1000000, 'side': 'buy', 'slippage_bps': 100,
                    }
                    status, buy_quote = request(base, token, '/v1/routes/quote', buy_request)
                    if status == 200:
                        break
                    assert 'expired' in buy_quote.get('error', '').lower(), (status, buy_quote)
                assert status == 200 and buy_quote['expected_output_units'] > 0, (status, buy_quote)
                assert buy_quote['input_mint'] == amm_hint['quote_mint'], buy_quote
                assert buy_quote['output_mint'] == amm_hint['mint'], buy_quote
                assert buy_quote['min_output_units'] == buy_quote['expected_output_units'] * 9900 // 10000, buy_quote
                assert buy_quote['expires_at_ms'] <= amm_route['expires_at_ms'], buy_quote
                for _ in range(3):
                    status, amm_route = request(base, token, '/v1/pools/resolve-candidates', amm_hint)
                    assert status == 200, (status, amm_route)
                    sell_request = dict(buy_request, route=amm_route,
                                        input_amount_units=1000, side='sell')
                    status, sell_quote = request(base, token, '/v1/routes/quote', sell_request)
                    if status == 200:
                        break
                    assert 'expired' in sell_quote.get('error', '').lower(), (status, sell_quote)
                assert status == 200 and sell_quote['expected_output_units'] > 0, (status, sell_quote)
                assert sell_quote['input_mint'] == amm_hint['mint'], sell_quote
                assert sell_quote['output_mint'] == amm_hint['quote_mint'], sell_quote
                bad_hint = dict(amm_hint, mint='11111111111111111111111111111111')
                status, _ = request(base, token, '/v1/pools/resolve-candidates', bad_hint)
                assert status == 502, f'AMM v4 pool accepted an unrelated target mint: {status}'
                assert count(db, 'trades') == count(db, 'aggregator_quotes') == count(db, 'aggregator_submissions') == 0
                amm_v4_quote_status = 'sdk_exact_in_both_sides'
            if args.positive_amm_v4_bridge_fixture:
                ray_usdc = dict(trade,
                    pool='raydium-amm-v4',
                    mint='4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R',
                    pool_id='6UmmUiYoBjSrhakAobJw8BvkmJtDVxaeBtbt7rxWo1mg',
                    quote_mint='EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
                    amount_lamports=1_000_000,
                    idempotency_key='ray-usdc-amm-v4-bridge')
                for attempt in range(3):
                    status, ray_decision = request(base, token, '/v1/routes/decide', ray_usdc)
                    if status == 200 and ray_decision.get('observed_bridge'):
                        break
                    assert status == 200 and ray_decision.get('reason') in (
                        'bridge_route_not_resolved', 'route_verification_unavailable'
                    ), (status, ray_decision)
                    if attempt < 2:
                        time.sleep(1)
                assert ray_decision['status'] == 'unresolved_retryable', ray_decision
                assert ray_decision['decision_id'] is None, ray_decision
                observation = ray_decision['observed_bridge']
                assert observation['target']['pool_id'] == ray_usdc['pool_id'], observation
                assert observation['target']['quote_mint'] == ray_usdc['quote_mint'], observation
                assert observation['sol_hop']['protocol_id'] == 'raydium-cpmm', observation
                assert ray_decision['sdk_capability']['service_adapter'] == 'cpmm_amm_v4_simulate_only', ray_decision
                status, unsigned_simulation = request(base, token, '/v1/routes/observations/sdk-preview', {
                    'observation_id': observation['observation_id'], 'trade': ray_usdc,
                    'simulate_transaction': True,
                })
                assert status == 200, (status, unsigned_simulation)
                simulated = unsigned_simulation['preview']['simulation']
                assert simulated['attempted'] is True and simulated['rpc_accepted'] is True, simulated
                assert simulated['transaction_bytes'] > 0, simulated
                assert unsigned_simulation.get('preview_id') is None and unsigned_simulation['executable'] is False, unsigned_simulation
                status, ray_preview = request(base, token, '/v1/routes/observations/sdk-preview', {
                    'observation_id': observation['observation_id'], 'trade': ray_usdc,
                    'simulate_transaction': False,
                })
                assert status == 200, (status, ray_preview)
                preview = ray_preview['preview']
                assert preview['builder'] == 'sol-trade-sdk-11:stonkfun-quote-route', preview
                assert preview['bridge_min_output_units'] > 0, preview
                assert preview['target_min_output_units'] > 0, preview
                assert preview['instruction_count'] >= 2, preview
                assert preview['snapshot_slot_span'] <= 64, preview
                assert 'CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C' in preview['program_ids'], preview
                assert '675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8' in preview['program_ids'], preview
                assert preview.get('simulation') is None, preview
                assert ray_preview.get('preview_id') is None and ray_preview['executable'] is False, ray_preview
                assert count(db, 'trades') == count(db, 'aggregator_quotes') == count(db, 'aggregator_submissions') == 0
                amm_v4_bridge_status = 'readonly_unsigned_simulation'
            if args.positive_pumpswap_fixture:
                positive = dict(trade,
                    pool='pump-amm',
                    mint='pumpCmXqMfrsAkQ5r49WcJnRayYRqmXz6ae8H7H9Dfn',
                    pool_id='539m4mVWt6iduB6W8rDGPMarzNCMesuqY5eUTiiYHAgR',
                    idempotency_key='positive-pumpswap-fixture')
                selected_pool_hint = {
                    'protocol_id': 'pump-amm',
                    'pool_id': positive['pool_id'],
                    'mint': positive['mint'],
                }
                status, selected_pool = request(
                    base, token, '/v1/pools/resolve-candidates', selected_pool_hint)
                assert status == 200, (status, selected_pool)
                assert (selected_pool['pool_id'] == positive['pool_id']
                        and selected_pool['mint'] == positive['mint']
                        and selected_pool['quote_mint'] == positive['quote_mint']
                        and selected_pool['chain_verified'] is True), selected_pool
                quote_request = {
                    'route': selected_pool,
                    'wallet_id': 'isolated',
                    'input_amount_units': 1000000,
                    'side': 'buy',
                    'slippage_bps': 100,
                }
                status, direct_quote = request(base, token, '/v1/routes/quote', quote_request)
                assert status == 200, (status, direct_quote)
                assert direct_quote['route_id'] == selected_pool['route_id'], direct_quote
                assert direct_quote['input_mint'] == selected_pool['quote_mint'], direct_quote
                assert direct_quote['output_mint'] == selected_pool['mint'], direct_quote
                assert direct_quote['expires_at_ms'] <= selected_pool['expires_at_ms'], direct_quote
                assert (direct_quote['expected_output_units'] > 0
                        and direct_quote['min_output_units'] ==
                        direct_quote['expected_output_units'] * 9900 // 10000
                        and direct_quote['fee_units'] > 0), direct_quote
                status, missing_quote = request(
                    base, token, '/v1/routes/quote', dict(quote_request, wallet_id='missing'))
                assert status == 404 and missing_quote['error'] == 'wallet not found', (
                    status, missing_quote)
                wrong_mint_hint = dict(selected_pool_hint, mint=positive['quote_mint'])
                status, _ = request(
                    base, token, '/v1/pools/resolve-candidates', wrong_mint_hint)
                assert status == 502, f'selected pool accepted an unrelated target mint: {status}'
                status, body = request(base, token, '/v1/routes/decide', positive)
                assert status == 200 and body['status'] == 'sdk_verified', (status, body)
                assert body.get('wallet_pubkey') == b58(pubkey), body
                assert body['decision_id'] and body['route_id'] and body['request_fingerprint'], body
                assert body['sdk_capability'] == {
                    'package_builder_family': 'PumpSwapParams',
                    'service_adapter': 'verified_direct_sdk',
                    'route_evidence': 'chain_verified_single_pool_route',
                    'sdk_unsupported_proven': False,
                    'jupiter_candidate_allowed': False,
                }, body
                positive_status = body['status']
                direct_without_quote = dict(positive, idempotency_key='pumpswap-quote-discovered')
                direct_without_quote.pop('quote_mint')
                status, discovered_direct = request(
                    base, token, '/v1/routes/decide', direct_without_quote)
                assert status == 200 and discovered_direct['status'] == 'sdk_verified', (
                    status, discovered_direct)
                assert discovered_direct['decision_id'] and discovered_direct['route_id'], discovered_direct
                assert discovered_direct['sdk_capability'] == body['sdk_capability'], discovered_direct
                mint_only_pumpswap = dict(positive, idempotency_key='positive-pumpswap-mint-only')
                mint_only_pumpswap.pop('pool_id')
                status, mint_only_body = request(base, token, '/v1/routes/decide', mint_only_pumpswap)
                assert status == 200, status
                mint_only_pumpswap_status = mint_only_body['status']
                if mint_only_pumpswap_status == 'sdk_verified':
                    assert mint_only_body['decision_id'] and mint_only_body['route_id'], mint_only_body
                else:
                    assert (mint_only_pumpswap_status == 'unresolved_retryable'
                            and mint_only_body['reason'] == 'route_verification_unavailable'
                            and mint_only_body['decision_id'] is None), (
                        mint_only_pumpswap_status, mint_only_body.get('reason'))
                tampered = dict(positive, sdk_decision_id=body['decision_id'],
                                amount_lamports=positive['amount_lamports'] + 1)
                status, _ = request(base, token, '/v1/trades', tampered)
                assert status == 400, f'tampered decision trade status={status}'
                bridge_intent = dict(trade,
                    pool='pump-amm',
                    mint='nxPqZ4F7ux8Vph5NFu25rEP6Kx9Gr1U6Fmj36Rjpump',
                    pool_id='G2UuRbmVDzDFP4qyoBJrSKBXSi7d6V4bPP9tHPs36y4b',
                    quote_mint='TTWofwAge91oFhZs7kpQdyrVRkmevgM88xijGvQFbKo',
                    amount_sol=0.001,
                    idempotency_key='ttwo-sol-bridge-intent')
                bridge_intent.pop('amount_lamports')
                bridge_without_quote = dict(
                    bridge_intent, idempotency_key='ttwo-quote-discovered')
                bridge_without_quote.pop('quote_mint')
                for discovery_attempt in range(3):
                    status, discovered_bridge = request(
                        base, token, '/v1/routes/decide', bridge_without_quote)
                    if (status == 200 and discovered_bridge.get('reason')
                            == 'bridge_verified_sdk_capability_unresolved'):
                        break
                    assert status == 200 and discovered_bridge.get('reason') in (
                        'bridge_route_not_resolved', 'route_verification_unavailable'
                    ), (status, discovered_bridge)
                    if discovery_attempt < 2:
                        time.sleep(1)
                assert discovered_bridge['status'] == 'unresolved_retryable', discovered_bridge
                assert discovered_bridge['reason'] == 'bridge_verified_sdk_capability_unresolved', discovered_bridge
                discovered_observation = discovered_bridge['observed_bridge']
                assert discovered_observation['target']['quote_mint'] == bridge_intent['quote_mint'], (
                    discovered_observation)
                assert discovered_bridge['decision_id'] is None, discovered_bridge
                status, discovered_preview = request(
                    base, token, '/v1/routes/observations/sdk-preview', {
                        'observation_id': discovered_observation['observation_id'],
                        'trade': bridge_without_quote,
                        'simulate_transaction': False,
                    })
                assert status == 200 and discovered_preview['preview']['instruction_count'] > 0, (
                    status, discovered_preview)
                assert discovered_preview.get('preview_id') is None, discovered_preview
                for bridge_attempt in range(3):
                    status, body = request(base, token, '/v1/routes/decide', bridge_intent)
                    if status == 200 and body.get('reason') == 'bridge_verified_sdk_capability_unresolved':
                        break
                    assert status == 200 and body.get('reason') in (
                        'bridge_route_not_resolved', 'route_verification_unavailable'
                    ), (status, body)
                    if bridge_attempt < 2:
                        time.sleep(1)
                assert status == 200 and body['status'] == 'unresolved_retryable', (status, body)
                bridge_aggregator = dict(bridge_intent,
                                         execution_method='aggregator',
                                         idempotency_key='ttwo-client-selected-jupiter')
                status, rejected_bridge_aggregator = request(base, token, '/v1/trades', bridge_aggregator)
                assert status == 409 and 'sdk_unsupported' in rejected_bridge_aggregator['error'], (
                    status, rejected_bridge_aggregator)
                assert count(db, 'trades') == 0, 'unresolved SDK bridge consumed an aggregator trade key'
                assert body['reason'] == 'bridge_verified_sdk_capability_unresolved', body
                assert body['decision_id'] is None and body['route_id'] is None, body
                capability = body['sdk_capability']
                assert capability == {
                    'package_builder_family': 'PumpSwapQuoteRoute',
                    'service_adapter': 'simulate_only_bridge_decision',
                    'route_evidence': 'chain_verified_bridge_pool_identity',
                    'sdk_unsupported_proven': False,
                    'jupiter_candidate_allowed': False,
                }, capability
                observation = body['observed_bridge']
                assert observation['observation_id'].startswith('observe:sol-bridge:'), observation
                assert observation['wallet_id'] == 'isolated' and observation['wallet_pubkey'] == b58(pubkey), observation
                assert len(observation['request_fingerprint']) == 64, observation
                target = observation['target']
                hop = observation['sol_hop']
                assert target['mint'] == bridge_intent['mint'] and target['pool_id'] == bridge_intent['pool_id'], observation
                assert target['quote_mint'] == bridge_intent['quote_mint'], observation
                assert target['input_mint'] == hop['output_mint'] == hop['mint'], observation
                assert hop['input_mint'] == hop['quote_mint'] == trade['quote_mint'], observation
                assert target['quote_token_program'] == hop['token_program'], observation
                assert observation['expires_at_ms'] == min(target['expires_at_ms'], hop['expires_at_ms']), observation
                assert observation['expires_at_ms'] > int(time.time() * 1000), observation
                old_observation_id = observation['observation_id']
                fake_bridge_decision = dict(bridge_intent, sdk_decision_id=old_observation_id)
                status, _ = request(base, token, '/v1/trades', fake_bridge_decision)
                assert status == 400, f'observation used as decision status={status}'
                assert count(db, 'trades') == 0, 'observation created a rejected trade before decision validation'
                preview_request = {'observation_id': old_observation_id, 'trade': bridge_intent,
                                   'simulate_transaction': True}
                tampered_preview = dict(preview_request, trade=dict(bridge_intent,
                                                                    amount_sol=0.002))
                status, _ = request(base, token, '/v1/routes/observations/sdk-preview',
                                    tampered_preview)
                assert status == 400, f'tampered SDK preview status={status}'
                status, preview_body = request(base, token, '/v1/routes/observations/sdk-preview',
                                               preview_request)
                assert status == 200 and preview_body['status'] == 'sdk_instruction_preview', (status, preview_body)
                assert preview_body['executable'] is False and preview_body['simulate_only'] is True, preview_body
                assert preview_body['observation_id'] == old_observation_id, preview_body
                assert preview_body.get('preview_id') is None, 'failed simulation issued a preview ID'
                preview = preview_body['preview']
                assert preview['builder'] == 'sol-trade-sdk-11:pumpswap-quote-route', preview
                assert preview['input_lamports'] == 1000000, preview
                assert preview['bridge_min_output_units'] > 0 and preview['target_min_output_units'] > 0, preview
                assert preview['instruction_count'] >= 2 and len(preview['program_ids']) >= 2, preview
                assert 'CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C' in preview['program_ids'], preview
                assert 'pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA' in preview['program_ids'], preview
                assert preview['validated_slot'] >= max(target['validated_slot'], hop['validated_slot']), preview
                assert 0 <= preview['snapshot_slot_span'] <= 64, preview
                simulation = preview['simulation']
                assert simulation['ok'] is False and simulation['error'], simulation
                assert simulation['slot'] >= preview['validated_slot'], simulation
                assert simulation['attempted'] is True, simulation
                assert simulation['rpc_accepted'] is True, simulation
                assert simulation['transaction_version'] in ('legacy', 'v1'), simulation
                if simulation['transaction_version'] == 'legacy':
                    assert simulation['attempted'] and 0 < simulation['transaction_bytes'] <= 1232, simulation
                else:
                    assert 1232 < simulation['transaction_bytes'] <= 4096, simulation
                sdk_preview_summary = {
                    'instruction_count': preview['instruction_count'],
                    'snapshot_slot_span': preview['snapshot_slot_span'],
                    'bridge_min_output_units': preview['bridge_min_output_units'],
                    'target_min_output_units': preview['target_min_output_units'],
                    'simulation_ok': simulation['ok'],
                    'simulation_attempted': simulation['attempted'],
                    'simulation_rpc_accepted': simulation['rpc_accepted'],
                    'simulation_error': simulation['error'],
                    'transaction_version': simulation['transaction_version'],
                    'transaction_bytes': simulation['transaction_bytes'],
                }
                bridge_status = body['reason']
            fake = dict(trade, sdk_decision_id='sdk-decision:fake')
            status, _ = request(base, token, '/v1/trades', fake)
            assert status == 400, f'fake decision trade status={status}'
            if args.positive_pumpswap_fixture:
                status, fresh_decision = request(base, token, '/v1/routes/decide', positive)
                assert status == 200 and fresh_decision['status'] == 'sdk_verified', (status, fresh_decision)
                old_decision_id = fresh_decision['decision_id']
                old_decision_expires_at_ms = fresh_decision['expires_at_ms']
                proc.terminate()
                proc.wait(timeout=5)
                proc = subprocess.Popen([str(binary)], cwd=root, env=env,
                                        stdout=log, stderr=subprocess.STDOUT)
                for _ in range(80):
                    if proc.poll() is not None:
                        raise RuntimeError(f'service exited after restart: {proc.returncode}')
                    try:
                        status, _ = request(base, None, '/health')
                        if status == 200:
                            break
                    except (OSError, ValueError):
                        pass
                    time.sleep(0.25)
                else:
                    raise RuntimeError('service did not become healthy after restart')
                assert int(time.time() * 1000) < old_decision_expires_at_ms, 'decision expired before restart check'
                same_trade = dict(positive, sdk_decision_id=old_decision_id)
                status, _ = request(base, token, '/v1/trades', same_trade)
                assert status == 400, f'old decision after restart status={status}'
                old_bridge = dict(bridge_intent, sdk_decision_id=old_observation_id)
                status, _ = request(base, token, '/v1/trades', old_bridge)
                assert status == 400, f'old observation after restart status={status}'
                assert count(db, 'trades') == 0, 'old observation created a trade after restart'
                if args.simulate_valid_decision:
                    time.sleep(3)  # let the isolated blockhash cache warm after restart
                status, body = request(base, token, '/v1/routes/decide', positive)
                assert status == 200 and body['status'] == 'sdk_verified', (status, body)
                assert body['decision_id'] != old_decision_id, body
                restart_status = 'old_decision_rejected_new_decision_verified'
            counts = {table: count(db, table) for table in
                      ('trades', 'aggregator_quotes', 'aggregator_submissions')}
            assert all(value == 0 for value in counts.values()), counts
            valid_decision_simulation = None
            valid_decision_error_kind = None
            if args.simulate_valid_decision:
                valid_trade = dict(positive, sdk_decision_id=body['decision_id'])
                status, result = request(base, token, '/v1/trades', valid_trade)
                assert status in (200, 202) and result['request_id'], status
                for _ in range(80):
                    status, result = request(base, token,
                                             '/v1/trades/' + result['request_id'])
                    assert status == 200, status
                    if result['status'] in ('simulated', 'failed', 'rejected', 'unknown'):
                        break
                    time.sleep(0.25)
                assert result['status'] in ('simulated', 'failed'), (
                    'valid SDK simulation did not pass its route and blockhash gates', result['status'])
                assert result['signature'] is None, 'simulate-only decision returned a signature'
                assert 'decision wallet is disabled or has changed' not in (result['error'] or ''), (
                    'file-wallet decision was rejected by database-only wallet check')
                assert count(db, 'trades') == 1 and count(db, 'aggregator_submissions') == 0
                valid_decision_simulation = result['status']
                error_text = result['error'] or ''
                valid_decision_error_kind = (
                    'blockhash_unavailable' if 'blockhash cache is empty or stale' in error_text else
                    'decision_binding' if 'SDK decision' in error_text else
                    'route_check' if 'route' in error_text.lower() else
                    'other' if error_text else None)
            print(json.dumps({'passed': True, 'unauthorized': 401,
                              'missing_wallet': 'policy_blocked',
                              'aggregator_request': 'policy_blocked',
                              'aggregator_trade_without_sdk_unsupported': 409,
                              'pump_token_quote_credential_trade': 400,
                              'pump_usdc_probe': pump_usdc_probe,
                              'rpc_unavailable': 'unresolved_retryable',
                              'positive_fixture': positive_status,
                              'pumpswap_direct_quote': 'sdk_exact_in' if positive_status else None,
                              'amm_v4_direct_quote': amm_v4_quote_status,
                              'amm_v4_bridge_preview': amm_v4_bridge_status,
                              'quote_discovery': 'wsol_sdk_verified_non_wsol_bridge_observed' if positive_status else None,
                              'mint_only_pumpswap': mint_only_pumpswap_status,
                              'non_wsol_sol_bridge': bridge_status,
                              'restart': restart_status,
                              'tampered_decision_trade': 400 if positive_status else None,
                              'observation_as_decision_trade': 400 if bridge_status else None,
                              'sdk_two_hop_instruction_preview': sdk_preview_summary,
                              'fake_decision_trade': 400,
                              'valid_decision_simulation': valid_decision_simulation,
                              'valid_decision_error_kind': valid_decision_error_kind,
                              'records': counts}, sort_keys=True))
        except Exception:
            if args.rpc_url_file:
                print('service log omitted because RPC credentials may be present')
                for line in (root / 'service.log').read_text().splitlines():
                    if 'pool resolution RPC lane failed' in line:
                        kind = ('timeout' if 'timed out' in line else
                                'pool_not_found' if 'No pool found' in line else 'other')
                        print('pool_resolver_failure=' + kind)
                    if 'SDK two-hop build preview unavailable' in line:
                        print('sdk_two_hop_preview_unavailable=true')
            else:
                print((root / 'service.log').read_text()[-2000:])
            raise
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
            log.close()


if __name__ == '__main__':
    main()
