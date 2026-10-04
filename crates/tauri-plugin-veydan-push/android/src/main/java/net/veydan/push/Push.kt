// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

package net.veydan.push

/**
 * One push, as the server sent it (services/push/spec/protocol.md, payload
 * version 2). The server says what kind of thing came and carries the
 * event when it fits; every word shown comes from this phone. Everything
 * here came from outside and is treated so: what does not look right is
 * dropped.
 */
internal data class Push(
  val version: Int,
  val type: String,
  /** The outer event as JSON, when the push could carry it. */
  val event: String?,
  val eventId: String?,
  val relay: String?,
  val groupId: String?,
  /** How many events this push stands for; more than one after a burst. */
  val count: Int,
  val trace: String,
  /** Payload of a service push, as JSON. */
  val data: String?,
) {
  /** Handled, not shown. */
  val silent: Boolean
    get() = type == TYPE_MANIFEST

  /** About messages, as opposed to a word from the server itself. */
  val aboutMessage: Boolean
    get() = type == TYPE_DM || type == TYPE_GROUP || type == TYPE_SYNC

  /** The map the messenger's core reads; the same keys the server sent. */
  fun asData(): Map<String, String> = buildMap {
    put("v", version.toString())
    put("type", type)
    event?.let { put("event", it) }
    eventId?.let { put("event_id", it) }
    relay?.let { put("relay", it) }
    groupId?.let { put("group_id", it) }
    if (count > 1) put("count", count.toString())
    put("trace", trace)
  }

  companion object {
    /** The version of the format this code understands. */
    const val VERSION = 2

    const val TYPE_DM = "dm"
    const val TYPE_GROUP = "group"
    /** More came than the server pushes one by one: `count` of them, and no word of which. */
    const val TYPE_SYNC = "sync"
    const val TYPE_TEST = "test"
    const val TYPE_BROADCAST = "broadcast"
    const val TYPE_MANIFEST = "manifest_update"

    private val TYPE = Regex("^[a-z_]{1,32}$")
    private val HEX64 = Regex("^[0-9a-f]{64}$")
    private val TRACE = Regex("^[0-9A-Za-z_-]{1,64}$")
    private val CHAT = Regex("^(dm|group):[0-9a-f]{64}$")

    fun from(data: Map<String, String>): Push? {
      val type = data["type"]?.takeIf { TYPE.matches(it) } ?: return null
      return Push(
        version = data["v"]?.toIntOrNull() ?: 1,
        type = type,
        event = data["event"]?.takeIf { it.isNotEmpty() && it.length <= 8192 },
        eventId = data["event_id"]?.takeIf { HEX64.matches(it) },
        relay = data["relay"]?.takeIf { it.startsWith("wss://") || it.startsWith("ws://") },
        groupId = data["group_id"]?.takeIf { HEX64.matches(it) },
        count = data["count"]?.toIntOrNull()?.takeIf { it in 1..9999 } ?: 1,
        trace = data["trace"]?.takeIf { TRACE.matches(it) } ?: "-",
        data = data["data"],
      )
    }

    /** `dm:<pubkey>` or `group:<id>`: what a notification is tagged with and a tap opens. */
    fun isChatKey(key: String): Boolean = CHAT.matches(key)
  }
}
