import unittest

import orjson

from first_two_wallet_copy import FirstTwoTracker, buy_traders, decode_frames


class TrackerTests(unittest.TestCase):
    def event(self, signature, action="buy", traders=("B",), mint="M", **extra):
        return {"mint": mint, "signature": signature, "action": action,
                "breakdown": [{"action": "buy", "trader": t} for t in traders], **extra}

    def test_creation_initial_buy_counts_and_second_buy_hits_once(self):
        tracker = FirstTwoTracker("A")
        self.assertEqual(tracker.observe(self.event("s1", "create"))["order"], 1)
        hit = tracker.observe(self.event("s2", traders=("A",)))
        self.assertEqual(hit["order"], 2)
        self.assertTrue(hit["wallet_a_hit"])
        self.assertIsNone(tracker.observe(self.event("s3", traders=("A",))))

    def test_third_buy_is_not_eligible(self):
        tracker = FirstTwoTracker("A")
        tracker.observe(self.event("create", "create", ()))
        tracker.observe(self.event("s1"))
        tracker.observe(self.event("s2"))
        self.assertIsNone(tracker.observe(self.event("s3", traders=("A",))))

    def test_old_mint_does_not_get_false_first_rank(self):
        row = FirstTwoTracker("A").observe(self.event("s1", traders=("A",)))
        self.assertEqual(row["type"], "wallet_buy_unranked")
        self.assertNotIn("order", row)
        self.assertFalse(row["wallet_a_hit"])

    def test_other_event_same_signature_can_still_reveal_wallet(self):
        tracker = FirstTwoTracker("A")
        tracker.observe(self.event("s1", "create"))
        tracker.observe(self.event("s2"))
        hit = tracker.observe(self.event("s2", traders=("A",), poolId="other-pool"))
        self.assertEqual(hit["order"], 2)
        self.assertTrue(hit["wallet_a_hit"])

    def test_seller_or_relayer_is_not_a_buyer(self):
        event = self.event("s", txSigner="A", tradersInvolved={"A": {}, "B": {}})
        event["breakdown"].append({"action": "sell", "trader": "A"})
        self.assertEqual(buy_traders(event), {"B"})

    def test_net_sell_with_buy_breakdown_is_counted(self):
        tracker = FirstTwoTracker("A")
        tracker.observe(self.event("create", "create", ()))
        hit = tracker.observe(self.event("s1", "sell", ("A",)))
        self.assertTrue(hit["wallet_a_hit"])

    def test_mints_have_separate_counters(self):
        tracker = FirstTwoTracker("A")
        tracker.observe(self.event("s1", "create"))
        row = tracker.observe(self.event("s1", "create", ("A",), mint="N"))
        self.assertEqual(row["order"], 1)
        self.assertTrue(row["wallet_a_hit"])

    def test_multipart_normalized_copy_is_ignored(self):
        event = self.event("s1")
        raw = orjson.dumps(event)
        self.assertEqual(decode_frames([raw]), event)
        self.assertIsNone(decode_frames([b"normalized", raw]))
        self.assertIsNone(decode_frames([b"bad-json"]))


if __name__ == "__main__":
    unittest.main()
