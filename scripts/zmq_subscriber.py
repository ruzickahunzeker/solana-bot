#!/usr/bin/env python3
import argparse
import sys

import zmq


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", required=True)
    args = parser.parse_args()

    ctx = zmq.Context.instance()
    sock = ctx.socket(zmq.SUB)
    sock.setsockopt(zmq.SUBSCRIBE, b"")
    sock.connect(args.endpoint)

    while True:
        frames = sock.recv_multipart()
        if len(frames) == 1:
            payload = frames[0]
        elif len(frames) == 2 and frames[0] == b"normalized":
            payload = frames[1]
        else:
            continue
        sys.stdout.buffer.write(payload)
        sys.stdout.buffer.write(b"\n")
        sys.stdout.buffer.flush()


if __name__ == "__main__":
    raise SystemExit(main())
