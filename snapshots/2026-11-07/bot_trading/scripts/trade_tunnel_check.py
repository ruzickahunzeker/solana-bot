"""Read-only tunnel readiness and optional owner wallet balance verification."""
import argparse
import json
import time
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--attempts', type=int, default=1)
    parser.add_argument('--owner-id', type=int)
    args = parser.parse_args()
    env = {}
    for line in (Path(__file__).resolve().parent.parent / '.env').read_text().splitlines():
        if '=' in line and not line.lstrip().startswith('#'):
            key, value = line.split('=', 1)
            env[key.strip()] = value.strip().strip('"').strip("'")
    base = env.get('TRADE_SERVICE_URL', '').rstrip('/')
    token = env.get('TRADE_SERVICE_TOKEN')
    if base != 'http://127.0.0.1:18788':
        raise SystemExit('TRADE_SERVICE_URL must be http://127.0.0.1:18788 on the Bot host; 18787 is the remote trade service port.')
    if not token:
        raise SystemExit('TRADE_SERVICE_TOKEN is missing or empty.')
    opener = build_opener(ProxyHandler({}))

    def get(path):
        request = Request(base + path, headers={'Authorization': 'Bearer ' + token})
        with opener.open(request, timeout=3) as response:
            if response.status != 200:
                raise ValueError('Unexpected HTTP status')
            return json.load(response)

    for attempt in range(max(1, args.attempts)):
        try:
            get('/health')
            ready = get('/ready')
            if ready.get('ready') is not True:
                raise ValueError('Trade service is not ready')
            wallets = get('/v1/wallets/manage')['wallets']
            print(json.dumps({'health': 'ok', 'ready': True,
                              'managed_wallet_count': len(wallets)}), flush=True)
            if args.owner_id is not None:
                owned = [w for w in wallets if w.get('enabled') and (
                    (w.get('owner_type') == 'system' and str(w.get('owner_id')) == 'global') or
                    (w.get('owner_type') == 'copy_trading' and str(w.get('owner_id')) == str(args.owner_id)))]
                balances = get('/internal/wallet-balances?' + urlencode({
                    'owner_type': 'copy_trading', 'owner_id': args.owner_id}))['wallets']
                usable = sum(w.get('balance_lamports') is not None and not w.get('error')
                             for w in balances)
                print(json.dumps({'owner_id': args.owner_id, 'visible_wallet_count': len(owned),
                                  'balance_wallet_count': len(balances), 'usable_balances': usable}), flush=True)
                if not owned or not balances or usable != len(balances):
                    raise ValueError('Owner wallet balance acceptance failed')
            return
        except HTTPError as error:
            reason = 'HTTP ' + str(error.code)
        except (URLError, TimeoutError, OSError):
            reason = 'Connection failed or timed out'
        except (ValueError, KeyError, TypeError):
            reason = 'Readiness or response validation failed'
        print(json.dumps({'attempt': attempt + 1, 'result': reason}), flush=True)
        if attempt + 1 < max(1, args.attempts):
            time.sleep(2)
    raise SystemExit('Tunnel acceptance failed; no transactions submitted.')


if __name__ == '__main__':
    main()
