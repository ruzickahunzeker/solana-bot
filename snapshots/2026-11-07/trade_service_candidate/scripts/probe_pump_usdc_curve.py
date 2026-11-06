#!/usr/bin/env python3
"""Find real Pump.fun V2 trades whose instruction quote mint is USDC.

Read-only RPC probe. It never loads a wallet or submits a transaction.
"""

import argparse
import json
import time
import urllib.error
import urllib.request
from pathlib import Path


PUMP_PROGRAM = '6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P'
USDC_MINT = 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'
BUY_V2 = bytes([184, 23, 238, 97, 103, 197, 211, 61])
SELL_V2 = bytes([93, 246, 130, 60, 231, 233, 64, 178])
BASE58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def decode_base58(value):
    number = 0
    for char in value:
        number = number * 58 + BASE58.index(char)
    body = number.to_bytes((number.bit_length() + 7) // 8, 'big') if number else b''
    return b'\0' * (len(value) - len(value.lstrip('1'))) + body


def rpc(url, payload):
    for attempt in range(5):
        try:
            request = urllib.request.Request(
                url, data=json.dumps(payload).encode(), headers={'content-type': 'application/json'})
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code != 429 or attempt == 4:
                raise RuntimeError(f'RPC HTTP status {error.code}') from None
            time.sleep(min(2 ** attempt, 8))
    raise RuntimeError('RPC retry limit reached')


def instructions(transaction):
    yield from transaction['transaction']['message']['instructions']
    for entry in transaction.get('meta', {}).get('innerInstructions') or []:
        yield from entry['instructions']


def matches(transaction):
    found = []
    for instruction in instructions(transaction):
        if instruction.get('programId') != PUMP_PROGRAM:
            continue
        accounts = instruction.get('accounts') or []
        data = instruction.get('data')
        if not data or len(accounts) < 11 or accounts[2] != USDC_MINT:
            continue
        discriminator = decode_base58(data)[:8]
        if discriminator in (BUY_V2, SELL_V2):
            found.append({
                'side': 'buy_v2' if discriminator == BUY_V2 else 'sell_v2',
                'mint': accounts[1], 'quote_mint': accounts[2], 'curve': accounts[10],
            })
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rpc-url-file', type=Path, required=True)
    parser.add_argument('--limit', type=int, default=100)
    parser.add_argument('--address', default=PUMP_PROGRAM,
                        help='read signatures for this public account, then inspect Pump V2 instructions')
    parser.add_argument('--before', help='signature cursor from an earlier scan')
    args = parser.parse_args()
    if not 1 <= args.limit <= 1000:
        parser.error('--limit must be between 1 and 1000')
    url = next((line.split('=', 1)[1].strip().strip('"\'')
                for line in args.rpc_url_file.read_text().splitlines()
                if line.startswith('RPC_URL=')), None)
    if not url:
        parser.error('RPC_URL is missing from --rpc-url-file')
    options = {'limit': args.limit}
    if args.before:
        options['before'] = args.before
    result = rpc(url, {'jsonrpc': '2.0', 'id': 1, 'method': 'getSignaturesForAddress',
                       'params': [args.address, options]})
    if result.get('error'):
        raise RuntimeError(f"signature RPC error code {result['error'].get('code')}")
    signatures = result['result']
    scanned = 0
    candidates = []
    unavailable = 0
    for start in range(0, len(signatures), 10):
        batch = [
            {'jsonrpc': '2.0', 'id': index, 'method': 'getTransaction',
             'params': [signatures[index]['signature'],
                        {'encoding': 'jsonParsed', 'maxSupportedTransactionVersion': 1}]}
            for index in range(start, min(start + 10, len(signatures)))
        ]
        response = rpc(url, batch)
        if not isinstance(response, list):
            raise RuntimeError('transaction RPC batch response was not a list')
        for item in response:
            index = item['id']
            transaction = item.get('result')
            if not transaction:
                unavailable += 1
                continue
            scanned += 1
            for match in matches(transaction):
                match.update(signature=signatures[index]['signature'], slot=transaction['slot'])
                candidates.append(match)
        time.sleep(0.5)
    print(json.dumps({
        'scan_address': args.address, 'signature_count': len(signatures),
        'transactions_read': scanned,
        'unavailable': unavailable, 'candidates': candidates,
        'next_before': signatures[-1]['signature'] if signatures else None,
    }, sort_keys=True))


if __name__ == '__main__':
    main()
