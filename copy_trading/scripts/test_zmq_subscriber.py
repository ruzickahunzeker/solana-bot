import unittest

from zmq_subscriber import payload_from_frames


class SubscriberFrameTests(unittest.TestCase):
    def test_accepts_requested_observation_topic(self):
        self.assertEqual(
            payload_from_frames([b"observation", b"{}"], "observation"), b"{}"
        )

    def test_does_not_cross_deliver_other_topics(self):
        self.assertIsNone(payload_from_frames([b"normalized", b"{}"], "observation"))

    def test_keeps_legacy_single_frame_payload(self):
        self.assertEqual(payload_from_frames([b"{}"], "observation"), b"{}")
