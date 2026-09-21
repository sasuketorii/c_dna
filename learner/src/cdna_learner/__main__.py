from __future__ import annotations
import json
import os
import signal
import struct
import sys
from .contracts import ContractError, MAX_FRAME, loads_strict

cancelled=False

def on_cancel(*_):
    global cancelled
    cancelled=True


def read_exact(n: int) -> bytes:
    chunks=[]
    while n:
        chunk=sys.stdin.buffer.read(n)
        if not chunk:
            raise ContractError('TRUNCATED_FRAME')
        chunks.append(chunk);n-=len(chunk)
    return b''.join(chunks)


def run() -> int:
    # Native thread limits are set before importing NumPy/sklearn/LightGBM.
    for key in ('OMP_NUM_THREADS','OPENBLAS_NUM_THREADS','MKL_NUM_THREADS','POLARS_MAX_THREADS'):
        os.environ.setdefault(key,'2')
    signal.signal(signal.SIGTERM,on_cancel)
    try:
        header=sys.stdin.buffer.read(4)
        if not header:
            return 0
        if len(header)!=4:
            raise ContractError('TRUNCATED_FRAME')
        size=struct.unpack('>I',header)[0]
        if size>MAX_FRAME:
            raise ContractError('INPUT_TOO_LARGE')
        request=loads_strict(read_exact(size))
        if not isinstance(request,dict) or set(request)-{'operation','snapshot','previous'}:
            raise ContractError('INPUT_INVALID')
        from .train import train, incremental
        if request.get('operation')=='train':
            result=train(request['snapshot'],cancelled=lambda:cancelled)
        elif request.get('operation')=='incremental':
            result=incremental(request['snapshot'],request['previous'])
        else:
            raise ContractError('INPUT_INVALID')
        response={'ok':True,'result':result}
    except ContractError as e:
        response={'ok':False,'error':{'code':e.code}}
    except Exception:
        # No traceback, input, local paths, or free-form exception message on wire.
        response={'ok':False,'error':{'code':'TRAINING_FAILED'}}
    raw=json.dumps(response,ensure_ascii=False,allow_nan=False,separators=(',',':')).encode()
    if len(raw)>MAX_FRAME:
        raw=b'{"ok":false,"error":{"code":"OUTPUT_TOO_LARGE"}}'
    sys.stdout.buffer.write(struct.pack('>I',len(raw))+raw)
    sys.stdout.buffer.flush()
    return 0 if response['ok'] else 1

if __name__=='__main__':
    raise SystemExit(run())
