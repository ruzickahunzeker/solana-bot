"""Tunnel acceptance: quotes and non-broadcast simulation only; no submit path."""
import json
import sqlite3
import time
import uuid
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

root = Path(__file__).resolve().parent.parent
cfg = {}
for line in (root / '.env').read_text().splitlines():
    if '=' in line and not line.lstrip().startswith('#'):
        key, value = line.split('=', 1)
        cfg[key.strip()] = value.strip().strip('"').strip("'")
base = cfg['TRADE_SERVICE_URL'].rstrip('/')
assert base == 'http://127.0.0.1:18788'
headers = {'Authorization': 'Bearer ' + cfg['TRADE_SERVICE_TOKEN'], 'Content-Type': 'application/json'}
opener = build_opener(ProxyHandler({}))

def call(path, body=None):
    assert path in ['/health', '/v1/wallets/manage', '/internal/conversions/wsol/quotes', '/internal/conversions/wsol/simulate']
    request = Request(base + path, headers=headers, data=None if body is None else json.dumps(body).encode())
    try:
        with opener.open(request, timeout=60) as response:
            return json.load(response)
    except HTTPError as error:
        raise RuntimeError('HTTP ' + str(error.code)) from None

assert call('/health').get('ok') is True
remote = [w for w in call('/v1/wallets/manage')['wallets'] if w['wallet_id'] == '2TE']
assert len(remote) == 1
wallet = remote[0]
assert wallet['enabled']
connection = sqlite3.connect('file:' + str(root / 'data/bot_trading.sqlite') + '?mode=ro', uri=True)
connection.row_factory = sqlite3.Row
local = connection.execute('SELECT id,user_id,public_key FROM wallets WHERE external_wallet_id=? AND is_deleted=0', ('2TE',)).fetchall()
assert local
for row in local:
    assert row['public_key'] == wallet['pubkey']
    assert (wallet['owner_type'] == 'system' and wallet['owner_id'] == 'global') or (wallet['owner_type'] == 'copy_trading' and str(wallet['owner_id']) == str(row['user_id']))
binding = {key: wallet[key] for key in ['wallet_id', 'owner_type', 'owner_id']}
results = []
for direction, amount, all_value in [('wrap', 1000000, False), ('unwrap', 1000000, False), ('unwrap', None, True)]:
    body = {**binding, 'direction': direction, 'all': all_value}
    if amount is not None:
        body['amount_lamports'] = amount
    response = call('/internal/conversions/wsol/quotes', body)
    quote = response['quote']
    assert isinstance(response['executable'], bool)
    required = ['quote_id', 'direction', 'amount_lamports', 'source_balance_lamports', 'estimated_network_fee_lamports', 'temporary_rent_lamports', 'rent_refund_lamports', 'expires_at_ms']
    assert all(key in quote for key in required)
    assert all(quote[key] == value for key, value in binding.items())
    assert quote['wallet_pubkey'] == wallet['pubkey']
    assert quote['direction'] == direction and isinstance(quote['amount_lamports'], int) and quote['amount_lamports'] > 0
    assert amount is None or amount == quote['amount_lamports']
    assert quote['expires_at_ms'] > int(time.time() * 1000)
    simulated = call('/internal/conversions/wsol/simulate', {**binding, 'quote_id': quote['quote_id'], 'simulate': True, 'idempotency_key': 'bot-acceptance-' + str(uuid.uuid4())})
    assert simulated['status'] == 'simulated' and simulated['quote_id'] == quote['quote_id']
    result = {'direction': direction, 'all': all_value, 'amount_lamports': quote['amount_lamports'], 'quote_executable': response['executable'], 'status': simulated['status'], 'same_quote_id': True, 'simulation_fields': list(simulated), 'cu': simulated.get('compute_units_consumed', simulated.get('compute_units', simulated.get('units_consumed')))}
    results.append(result)
    print(json.dumps(result), flush=True)
(root / 'state/wsol-tunnel-acceptance.json').write_text(json.dumps({'health': 'ok', 'wallet_id': '2TE', 'mapping_verified': True, 'results': results, 'broadcast_requested': False, 'submit_test_performed': False}, indent=2) + '\n')
