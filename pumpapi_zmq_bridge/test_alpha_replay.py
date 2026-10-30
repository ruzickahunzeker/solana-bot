import unittest

from alpha_replay import replay


def event(event_id, timestamp, action, signer, amount, route="executable"):
    return {
        "event_id": event_id,
        "event_time_ms": timestamp,
        "received_at_ms": timestamp + 5,
        "action": action,
        "mint": "mint-1",
        "event_pool": "pump",
        "trade_service_pool": "pump",
        "pool_id": None,
        "quote_amount": amount,
        "tx_signer": signer,
        "route_status": route,
    }


class AlphaReplayTests(unittest.TestCase):
    def test_replay_emits_paper_signal_after_flow_confirmation(self):
        events = [
            event("b1", 1_000, "buy", "a", 10),
            event("b2", 2_000, "buy", "b", 10),
            event("b3", 3_000, "buy", "c", 10),
            event("b4", 4_000, "buy", "d", 10),
        ]
        signals, stats = replay(events, min_score=0.0, min_traders=3)
        self.assertEqual(stats["events_read"], 4)
        self.assertGreaterEqual(len(signals), 1)
        self.assertEqual(signals[-1]["mode"], "paper")
        self.assertEqual(signals[-1]["mint"], "mint-1")

    def test_replay_deduplicates_and_ignores_observation_only(self):
        first = event("b1", 1_000, "buy", "a", 10)
        duplicate = dict(first)
        ignored = event("x1", 2_000, "buy", "b", 10, route="observable-only")
        signals, stats = replay([first, duplicate, ignored], min_score=0.0, min_traders=0)
        self.assertEqual(stats["events_duplicate"], 1)
        self.assertEqual(stats["events_observation_only"], 1)
        self.assertEqual(signals, [])


if __name__ == "__main__":
    unittest.main()
