package com.twitter.scarecrow.features;

import com.twitter.botmaker.FeatureModifier;
import com.twitter.birdwatch.thriftjava.BirdwatchStatusChangeEvent;
import com.twitter.birdwatch.thriftjava.BirdwatchStatusChangeEventNoteStatusChange;
import com.twitter.birdwatch.thriftjava.BirdwatchNoteRatingStatus;
import com.twitter.spam.botmaker_features.BotMakerFeatures;
import com.twitter.spam.botmaker_features.FeatureMapBuilder;
import com.twitter.spam.botmaker_features.FeatureMapExtractor;

public class FeaturesOfBirdwatchStatusChangeEvent extends FeatureMapExtractor {
  private final BirdwatchStatusChangeEvent event;

  public FeaturesOfBirdwatchStatusChangeEvent(BirdwatchStatusChangeEvent event) {
    this.event = event;
  }

  @Override
  public void apply(FeatureMapBuilder builder) throws Exception {
    if (!event.isSetTweetId()) {
      return;
    }

    boolean hasStatus = false;
    boolean visible = false;

    if (event.isSetAfterStatus()) {
      hasStatus = true;
      visible = isCurrentlyRatedHelpful(event.getAfterStatus());
    }

    if (event.isSetNoteStatusChanges()) {
      for (BirdwatchStatusChangeEventNoteStatusChange change : event.getNoteStatusChanges()) {
        if (!change.isSetAfterStatus()) {
          continue;
        }
        hasStatus = true;
        if (isCurrentlyRatedHelpful(change.getAfterStatus())) {
          visible = true;
        }
      }
    }

    if (!hasStatus) {
      return;
    }

    builder.putValue(FeatureModifier.REQUIRED, BotMakerFeatures.tweetId, event.getTweetId());
    builder.putValue(FeatureModifier.REQUIRED, BotMakerFeatures.communityNoteVisible, visible);
  }

  private static boolean isCurrentlyRatedHelpful(BirdwatchNoteRatingStatus status) {
    return status == BirdwatchNoteRatingStatus.CURRENTLY_RATED_HELPFUL;
  }
}
