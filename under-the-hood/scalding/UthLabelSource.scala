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
    val typeName = source.getClass.getSimpleName
    isGrokName(productName) || isGrokName(typeName)
  }

  private def isGrokName(name: String): Boolean = {
    val simple = name.split('.').last.split('$').last
    simple == "GrokAnnotationAction" || simple.startsWith("GrokAnnotation")
  }
}
