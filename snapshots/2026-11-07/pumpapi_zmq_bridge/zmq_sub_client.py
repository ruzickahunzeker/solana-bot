#!/usr/bin/env python3
import os
import sys

import orjson
import zmq


def main() -> int:
    endpoint = os.getenv("ZMQ_ENDPOINT", "tcp://127.0.0.1:8939")
    # Empty topic receives raw single-frame PumpAPI messages. Use "normalized"
    # to receive bridge-generated normalized multipart messages.
    topic = sys.argv[1] if len(sys.argv) > 1 else os.getenv("ZMQ_TOPIC", "")

    ctx = zmq.Context.instance()
    sub = ctx.socket(zmq.SUB)
    sub.connect(endpoint)
    sub.setsockopt_string(zmq.SUBSCRIBE, topic)
    print(f"subscribed {endpoint} topic={topic!r}", file=sys.stderr, flush=True)

    while True:
        parts = sub.recv_multipart()
        if len(parts) == 1:
            print(orjson.dumps(orjson.loads(parts[0])).decode(), flush=True)
        else:
            msg_topic, body = parts[0], parts[1]
            print(
                msg_topic.decode(errors="replace"),
                orjson.dumps(orjson.loads(body)).decode(),
                flush=True,
            )


if __name__ == "__main__":
    raise SystemExit(main())
