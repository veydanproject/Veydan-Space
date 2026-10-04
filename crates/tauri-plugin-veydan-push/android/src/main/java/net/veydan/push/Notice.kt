// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

import org.json.JSONObject

/** What the messenger's core made of a push (messenger-notify, `Outcome`). */
internal sealed class Outcome {
  /** A message to show in full: who, what, where. */
  data class Show(val notice: Notice) : Outcome()

  /** Something came; no more can or may be said. */
  data class Plain(val plain: PlainNotice) : Outcome()

  /** Nothing to show, and why. */
  data class Quiet(val reason: String) : Outcome()

  /** The core could not answer. */
  data class Error(val error: String) : Outcome()

  companion object {
    fun from(json: JSONObject): Outcome = when (json.optString("outcome")) {
      "show" -> Show(Notice.from(json))
      "plain" -> Plain(PlainNotice.from(json))
      "quiet" -> Quiet(json.optString("reason", "?"))
      else -> Error(json.optString("error", "unknown outcome"))
    }
  }
}

/**
 * The text under `key`, or null when there is none. `optString` gives the
 * word "null" for a JSON null, and a notification would show it.
 */
internal fun JSONObject.str(key: String): String? =
  if (isNull(key)) null else optString(key).takeIf { it.isNotEmpty() }

/** What a message was, without a word of any language (`messenger_core::Body`). */
internal sealed class Body {
  data class Text(val text: String) : Body()
  /** A message that is one link: `link` is `web`, `group` or `contact`; `title` what it names. */
  data class Link(val link: String, val title: String?) : Body()
  data class Media(
    val kind: String,
    val name: String?,
    val caption: String?,
    val durationMs: Long?,
    /** Files sent together carry one. */
    val batch: String?,
  ) : Body()
  data class Invite(val groupName: String) : Body()
  data class JoinRequest(val groupName: String) : Body()
  data class Welcome(val groupName: String) : Body()

  companion object {
    fun from(json: JSONObject?): Body? {
      json ?: return null
      return when (json.str("t")) {
        "text" -> json.str("text")?.let { Text(it) }
        "link" -> Link(json.str("link") ?: "web", json.str("title"))
        "media" -> Media(
          kind = json.str("kind") ?: "file",
          name = json.str("name"),
          caption = json.str("caption"),
          durationMs = json.optLong("duration_ms", 0).takeIf { it > 0 },
          batch = json.str("batch"),
        )
        "invite" -> Invite(json.str("group_name") ?: "")
        "join_request" -> JoinRequest(json.str("group_name") ?: "")
        "welcome" -> Welcome(json.str("group_name") ?: "")
        else -> null
      }
    }
  }
}

internal data class Notice(
  /** `dm` | `request` | `group` */
  val kind: String,
  val chat: String?,
  val title: String,
  val sender: String,
  val senderKey: String,
  val picture: String?,
  val body: Body?,
  val muted: Boolean,
  val hideOnLockscreen: Boolean,
  val count: Int,
) {
  companion object {
    fun from(json: JSONObject) = Notice(
      kind = json.str("kind") ?: "dm",
      chat = json.str("chat")?.takeIf { Push.isChatKey(it) },
      title = json.str("title") ?: "",
      sender = json.str("sender") ?: "",
      senderKey = json.str("sender_key") ?: "",
      picture = json.str("picture")?.takeIf { it.startsWith("https://") },
      body = Body.from(json.optJSONObject("body")),
      muted = json.optBoolean("muted", false),
      hideOnLockscreen = json.optBoolean("hide_on_lockscreen", false),
      count = json.optInt("count", 1).coerceIn(1, 9999),
    )
  }
}

internal data class PlainNotice(
  /** `dm` | `group` */
  val kind: String,
  val chat: String?,
  /** The group's name, when known. */
  val title: String?,
  val muted: Boolean,
  val count: Int,
) {
  companion object {
    fun from(json: JSONObject) = PlainNotice(
      kind = json.str("kind") ?: "dm",
      chat = json.str("chat")?.takeIf { Push.isChatKey(it) },
      title = json.str("title"),
      muted = json.optBoolean("muted", false),
      count = json.optInt("count", 1).coerceIn(1, 9999),
    )

    /** What is known of a push without the core: its kind and its group. */
    fun of(push: Push) = PlainNotice(
      kind = push.type,
      chat = push.groupId?.let { "group:$it" },
      title = null,
      muted = false,
      count = push.count,
    )
  }
}
