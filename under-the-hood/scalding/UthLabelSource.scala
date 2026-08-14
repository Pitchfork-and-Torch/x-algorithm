package com.twitter.visibility.under_the_hood

import com.twitter.spam.rtf.thriftscala.SafetyLabel
import com.twitter.spam.rtf.thriftscala.SafetyLabelSource

object UthLabelSource {

  val Automated = "automated"
  val Manual = "manual"
  val Llm = "llm"
  val Unknown = "unknown"

  def fromEventLabel(label: Option[SafetyLabel]): Option[String] =
    label.flatMap(_.safetyLabelSource).map(coarseCategory)

  /**
   * Public reportJson tokens: automated | manual | llm | unknown.
   * Does not emit rule_id, actor_ldap, agent_tool, or VF-client type names.
   *
   * The jobs compile against spam.rtf SafetyLabelSource. Published in-repo
   * usage of that IDL only names BotMakerAction and ToolAction, so those
   * map to automated/manual. GrokAnnotationAction exists on unpublished
   * xai_x_thrift (VF client), not this IDL — it is not matched here and
   * folds to unknown rather than inventing a case that may not compile.
   */
  private[under_the_hood] def coarseCategory(source: SafetyLabelSource): String =
    source match {
      case SafetyLabelSource.BotMakerAction(_) => Automated
      case SafetyLabelSource.ToolAction(_) => Manual
      case _ => Unknown
    }
}
