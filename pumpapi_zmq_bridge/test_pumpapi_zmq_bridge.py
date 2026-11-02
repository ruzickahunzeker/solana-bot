import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import pumpapi_zmq_bridge as bridge


class PumpApiBridgeTests(unittest.TestCase):
    def test_platform_classifies_stonkfun_without_known_config(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(
            bridge, "STONKFUN_MINTS", set()
        ), patch.dict(
            bridge.os.environ,
            {"STONKFUN_ROUTE_FILE": str(Path(directory) / "stonkfun.txt")},
        ):
            event = {
                "action": "buy",
                "pool": "raydium-launchpad",
                "platform": "stonkfun",
                "poolId": "pool",
                "mint": "meme",
                "quoteMint": "stock",
            }
            normalized = bridge.normalize_for_trade_service(event)
            self.assertEqual(normalized["trade_service_pool"], "stonkfun")
            self.assertEqual(normalized["quote_mint"], "stock")

    def test_custom_pool_without_quote_is_not_assumed_wsol(self):
        event = {
            "action": "buy",
            "pool": "raydium-cpmm",
            "poolId": "pool",
            "mint": "meme",
        }
        normalized = bridge.normalize_for_trade_service(event)
        self.assertEqual(normalized["trade_service_pool"], "raydium-cpmm")
        self.assertIsNone(normalized["quote_mint"])

    def test_legacy_pump_without_quote_keeps_wsol_compatibility(self):
        event = {"txType": "buy", "pool": "pump", "mint": "meme"}
        normalized = bridge.normalize_for_trade_service(event)
        self.assertEqual(normalized["quote_mint"], bridge.WSOL_MINT)
        self.assertEqual(normalized["action"], "buy")

    def test_untrusted_platform_name_is_ignored(self):
        event = {
            "action": "createConfig",
            "pool": "raydium-launchpad",
            "platformName": "stonkfun",
            "launchpadConfig": "untrusted",
            "mint": "meme",
        }
        self.assertIsNone(bridge.normalize_for_trade_service(event))

    def test_normalized_event_has_schema_and_stable_id(self):
        event = {
            "signature": "sig",
            "action": "buy",
            "pool": "pump",
            "mint": "meme",
            "quoteAmount": 10,
            "tokenAmount": 20,
        }
        normalized = bridge.normalize_for_trade_service(event)
        self.assertEqual(normalized["schema_version"], 1)
        self.assertEqual(normalized["event_id"], "sig:buy:meme:unknown")
        self.assertEqual(normalized["event_id"], bridge.event_id(event))

    def test_observation_keeps_unknown_pool_as_observable_only(self):
        event = {
            "signature": "sig",
            "action": "buy",
            "pool": "unknown-pool",
            "mint": "meme",
        }
        observation = bridge.normalize_observation(event, "hash", 123)
        self.assertEqual(observation["route_status"], "observable-only")
        self.assertIsNone(observation["trade_service_pool"])
        self.assertEqual(observation["received_at_ms"], 123)

    def test_observation_keeps_custom_pool_without_quote_observable_only(self):
        event = {
            "signature": "sig",
            "action": "buy",
            "pool": "raydium-cpmm",
            "poolId": "pool",
            "mint": "meme",
        }
        observation = bridge.normalize_observation(event, "hash", 123)
        self.assertEqual(observation["route_status"], "observable-only")
        self.assertIsNone(observation["quote_mint"])

    def test_unsigned_event_id_uses_content_hash(self):
        event = {"action": "buy", "pool": "pump", "mint": "meme"}
        first = bridge.event_id(event)
        second = bridge.event_id(dict(event))
        self.assertTrue(first.startswith("sha256:"))
        self.assertEqual(first, second)


if __name__ == "__main__":
    unittest.main()
