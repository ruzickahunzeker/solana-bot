#!/usr/bin/env python3
import argparse
import os
import sys

import zmq


def payload_from_frames(frames: list[bytes], topic: str) -> bytes | None:
    if len(frames) == 1:
        return frames[0]
    if len(frames) == 2 and frames[0].decode(errors="ignore") == topic:
        return frames[1]
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--endpoint", required=True)
    parser.add_argument(
        "--topic",
        default=os.getenv("ZMQ_TOPIC", "observation"),
        help="ZMQ topic; observation keeps the full Alpha observation stream",
    )
    args = parser.parse_args()

    ctx = zmq.Context.instance()
    sock = ctx.socket(zmq.SUB)
    sock.setsockopt_string(zmq.SUBSCRIBE, args.topic)
    sock.connect(args.endpoint)

    while True:
        frames = sock.recv_multipart()
        payload = payload_from_frames(frames, args.topic)
        if payload is None:
            continue
        sys.stdout.buffer.write(payload)
        sys.stdout.buffer.write(b"\n")
        sys.stdout.buffer.flush()


if __name__ == "__main__":
    raise SystemExit(main())
