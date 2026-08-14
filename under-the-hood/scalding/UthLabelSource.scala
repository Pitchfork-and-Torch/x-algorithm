package com.twitter.visibility.under_the_hood

import com.twitter.spam.rtf.thriftscala.SafetyLabel
import com.twitter.spam.rtf.thriftscala.SafetyLabelSource

object UthLabelSource {

  val Automated = "automated"
  val Manual = "manual"
  val Other = "other"

  def fromEventLabel(label: Option[SafetyLabel]): Option[String] =
    label.flatMap(_.safetyLabelSource).map(coarseCategory)

  /**
   * Maps the per-event SafetyLabelSource union to a public category.
   * Does not emit rule_id, actor_ldap, or agent_tool.
   *
   * BotMakerAction = automated systems; ToolAction = manual/tool apply.
   * Any other set variant (including LLM annotations if present on the IDL)
   * is "other" so unknown union members stay compile-safe.
   */
  private[under_the_hood] def coarseCategory(source: SafetyLabelSource): String =
    source match {
      case SafetyLabelSource.BotMakerAction(_) => Automated
      case SafetyLabelSource.ToolAction(_) => Manual
      case _ => Other
    }
}
