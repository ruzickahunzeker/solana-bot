"""Readiness checks and optional simulated trade; never submits a live trade."""
import argparse
import json
import time
import uuid
from pathlib import Path
from urllib.error import HTTPError,URLError
from urllib.request import Request,build_opener,ProxyHandler

def main():
    p=argparse.ArgumentParser()
    p.add_argument('--mint')
    p.add_argument('--wallet-id')
    p.add_argument('--amount-sol',type=float,default=0.001)
    args=p.parse_args()
    env={}
    for line in (Path(__file__).resolve().parent.parent/'.env').read_text().splitlines():
        if '=' in line and not line.lstrip().startswith('#'):
            key,value=line.split('=',1)
            env[key.strip()]=value.strip().strip('"').strip("'")
    base=env['TRADE_SERVICE_URL'].rstrip('/')
    token=env.get('TRADE_SERVICE_TOKEN')
    opener=build_opener(ProxyHandler({}))
    def call(path,body=None):
        headers={'Accept':'application/json'}
        if token: headers['Authorization']='Bearer '+token
        if body is not None: headers['Content-Type']='application/json'
        req=Request(base+path,data=json.dumps(body).encode() if body is not None else None,headers=headers)
        try:
            with opener.open(req,timeout=5) as response:
                data=json.load(response)
                return response.status,data
        except HTTPError as exc:
            return exc.code,{}
        except (URLError,TimeoutError,OSError):
            return 0,{}
    status,_=call('/health'); print(json.dumps({'check':'health','http':status}))
    if status==0: raise SystemExit('Remote service unreachable; no trade request sent.')
    if not token: raise SystemExit('TRADE_SERVICE_TOKEN missing; no trade request sent.')
    status,ready=call('/ready')
    print(json.dumps({'check':'ready','http':status,'ready':ready.get('ready')}))
    if status!=200 or not ready.get('ready'): raise SystemExit('Remote readiness failed; no trade request sent.')
    status,wallets=call('/v1/wallets')
    available=wallets.get('wallets',[])
    print(json.dumps({'check':'wallets','http':status,'wallet_count':len(available)}))
    if status!=200 or not args.mint: return
    wallet=args.wallet_id or ready.get('default_wallet_id')
    if not any(w.get('wallet_id')==wallet for w in available):
        raise SystemExit('Selected wallet is unavailable; no trade request sent.')
    if not 0<args.amount_sol<=0.001: raise SystemExit('Test amount must be >0 and <=0.001 SOL.')
    status,route=call('/v1/pools/resolve',{'mint':args.mint})
    if status!=200: raise SystemExit('Pool resolution failed; no trade request sent.')
    if route.get('quote_mint')!='So11111111111111111111111111111111111111112':
        raise SystemExit('Test requires explicit WSOL quote; no trade request sent.')
    request=dict(wallet_id=wallet,execution_method='sdk',side='buy',mint=args.mint,
        pool=route['pool'],pool_id=route.get('pool_id'),quote_mint=route['quote_mint'],
        amount_sol=args.amount_sol,slippage_percent=1,tip_sol=0,use_mev=False,simulate=True,
        source='bot-trading-smoke',idempotency_key='smoke-sim-'+uuid.uuid4().hex)
    status,result=call('/v1/trades',request)
    rid=result.get('request_id')
    print(json.dumps({'check':'simulate_submit','http':status,'request_id':rid,'status':result.get('status')}))
    if status not in (200,201,202) or not rid: raise SystemExit('Simulation submission failed.')
    for _ in range(15):
        status,final=call('/v1/trades/'+rid)
        if final.get('status') not in ('accepted','warming_params','building_tx','submitting'): break
        time.sleep(1)
    print(json.dumps({'check':'simulate_result','http':status,'status':final.get('status')}))
    if final.get('status')!='simulated': raise SystemExit('Simulation did not reach simulated status.')
    status,duplicate=call('/v1/trades',request)
    ok=duplicate.get('request_id')==rid and duplicate.get('duplicate') is True
    print(json.dumps({'check':'idempotency','http':status,'same_request':ok}))
    if not ok: raise SystemExit('Simulation idempotency check failed.')

if __name__=='__main__':
    main()
