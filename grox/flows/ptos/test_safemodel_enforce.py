import unittest

from grox.flows.ptos.safemodel_enforce import (
    should_apply_safemodel_nsfw_drop_labels,
)


class SafemodelEnforceTest(unittest.TestCase):
    def test_safemodel_only_positive_does_not_apply_drop_labels(self):
        self.assertFalse(
            should_apply_safemodel_nsfw_drop_labels(
                safemodel_positive=True, ptos_confirmed_hard_nsfw=False
            )
        )

    def test_ptos_hard_nsfw_already_labeled_does_not_need_safemodel(self):
        self.assertTrue(
            should_apply_safemodel_nsfw_drop_labels(
                safemodel_positive=True, ptos_confirmed_hard_nsfw=True
            )
        )

    def test_neither_positive_does_not_apply_drop_labels(self):
        self.assertFalse(
            should_apply_safemodel_nsfw_drop_labels(
                safemodel_positive=False, ptos_confirmed_hard_nsfw=False
            )
        )

    def test_ptos_only_does_not_mint_labels_from_safemodel(self):
        self.assertFalse(
            should_apply_safemodel_nsfw_drop_labels(
                safemodel_positive=False, ptos_confirmed_hard_nsfw=True
            )
        )


if __name__ == "__main__":
    unittest.main()
