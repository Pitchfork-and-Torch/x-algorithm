package com.twitter.visibility.under_the_hood

import com.twitter.spam.rtf.thriftscala.SafetyLabel
import com.twitter.spam.rtf.thriftscala.SafetyLabelSource

object UthLabelSource {

  val Automated = "automated"
  val Manual = "manual"
  val Llm = "llm"
  val Unknown = "unknown"

  def fromEventLabel(label: Option[SafetyLabel]): Option[String] =
    persistToken(label.flatMap(_.safetyLabelSource).flatMap(coarseCategory))

  /** Persist only automated | manual | llm. Unset/unmapped stay empty. */
  def persistToken(source: Option[String]): Option[String] =
    source.map(_.trim.toLowerCase) match {
      case Some(Automated) => Some(Automated)
      case Some(Manual) => Some(Manual)
      case Some(Llm) => Some(Llm)
      case Some(Unknown) | Some("other") | Some("unavailable") => None
      case _ => None
    }

  /**
   * Public reportJson tokens: automated | manual | llm | unknown.
   * Does not emit rule_id, actor_ldap, agent_tool, or VF-client type names.
   *
   * Named cases are only those present on published spam.rtf usage
   * (BotMakerAction, ToolAction). A Grok/LLM union member is detected by
   * simple class name so this compiles if that case is absent from the IDL.
   */
  private[under_the_hood] def coarseCategory(source: SafetyLabelSource): Option[String] =
    source match {
      case SafetyLabelSource.BotMakerAction(_) => Some(Automated)
      case SafetyLabelSource.ToolAction(_) => Some(Manual)
      case other if isLlmVariant(other) => Some(Llm)
      case _ => None
    }

  private def isLlmVariant(source: SafetyLabelSource): Boolean = {
    val productName = source match {
      case p: Product => p.productPrefix
      case _ => ""
    }
    val cls = source.getClass
    isGrokName(productName) || isGrokName(cls.getSimpleName) || isGrokName(cls.getName)
  }

  private def isGrokName(name: String): Boolean = {
    val simple = lastNonEmptySegment(name)
    simple == "GrokAnnotationAction" || simple.startsWith("GrokAnnotation")
  }

  // Last `.` / `$` segment, skipping a trailing synthetic `$` (Scala module suffix).
  // Do not String.split("$"): `$` is regex end-of-string.
  private def lastNonEmptySegment(name: String): String = {
    var end = name.length
    while (end > 0 && (name.charAt(end - 1) == '.' || name.charAt(end - 1) == '$')) {
      end -= 1
    }
    if (end == 0) ""
    else {
      val start = name.lastIndexOf('.', end - 1).max(name.lastIndexOf('$', end - 1))
      name.substring(start + 1, end)
    }
  }
}
