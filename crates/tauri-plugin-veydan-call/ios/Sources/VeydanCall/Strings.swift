// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

import Foundation

/**
 * The few words the plugin shows itself, in the app's 15 languages: the
 * same as the Kotlin side's (the strings.xml of android/src/main/res),
 * which are the source; a word changed there is changed here.
 *
 * They live in the code, not in a bundle of the package: the app links
 * this package as a static library, and a resource bundle of it would not
 * be copied into the app. The language is the first of the user's
 * preferred ones the table has, English otherwise.
 */
enum Strings {
  private struct Words {
    let incoming: String
    let incomingVideo: String
    let groupOn: String
    let groupVideoOn: String
  }

  private static let table: [String: Words] = [
    "en": Words(incoming: "Incoming call", incomingVideo: "Incoming video call", groupOn: "A call is on", groupVideoOn: "A video call is on"),
    "de": Words(incoming: "Eingehender Anruf", incomingVideo: "Eingehender Videoanruf", groupOn: "Ein Anruf läuft", groupVideoOn: "Ein Videoanruf läuft"),
    "es": Words(incoming: "Llamada entrante", incomingVideo: "Videollamada entrante", groupOn: "Hay una llamada en curso", groupVideoOn: "Hay una videollamada en curso"),
    "fr": Words(incoming: "Appel entrant", incomingVideo: "Appel vidéo entrant", groupOn: "Un appel est en cours", groupVideoOn: "Un appel vidéo est en cours"),
    "id": Words(incoming: "Panggilan masuk", incomingVideo: "Panggilan video masuk", groupOn: "Ada panggilan berlangsung", groupVideoOn: "Ada panggilan video berlangsung"),
    "it": Words(incoming: "Chiamata in arrivo", incomingVideo: "Videochiamata in arrivo", groupOn: "È in corso una chiamata", groupVideoOn: "È in corso una videochiamata"),
    "ja": Words(incoming: "着信", incomingVideo: "ビデオ通話の着信", groupOn: "通話が進行中です", groupVideoOn: "ビデオ通話が進行中です"),
    "ko": Words(incoming: "수신 전화", incomingVideo: "영상 통화 수신", groupOn: "통화가 진행 중입니다", groupVideoOn: "영상 통화가 진행 중입니다"),
    "pl": Words(incoming: "Połączenie przychodzące", incomingVideo: "Przychodzące połączenie wideo", groupOn: "Trwa połączenie", groupVideoOn: "Trwa połączenie wideo"),
    "pt": Words(incoming: "Chamada recebida", incomingVideo: "Chamada de vídeo recebida", groupOn: "Há uma chamada em andamento", groupVideoOn: "Há uma chamada de vídeo em andamento"),
    "ru": Words(incoming: "Входящий звонок", incomingVideo: "Входящий видеозвонок", groupOn: "Идёт звонок", groupVideoOn: "Идёт видеозвонок"),
    "tr": Words(incoming: "Gelen arama", incomingVideo: "Gelen görüntülü arama", groupOn: "Bir arama sürüyor", groupVideoOn: "Bir görüntülü arama sürüyor"),
    "uk": Words(incoming: "Вхідний дзвінок", incomingVideo: "Вхідний відеодзвінок", groupOn: "Триває дзвінок", groupVideoOn: "Триває відеодзвінок"),
    "vi": Words(incoming: "Cuộc gọi đến", incomingVideo: "Cuộc gọi video đến", groupOn: "Đang có cuộc gọi", groupVideoOn: "Đang có cuộc gọi video"),
    "zh": Words(incoming: "来电", incomingVideo: "视频来电", groupOn: "正在通话", groupVideoOn: "正在进行视频通话"),
  ]

  private static var words: Words {
    for language in Locale.preferredLanguages {
      // `pt-BR`, `zh-Hans-CN`: the language alone; Android's old code of
      // Indonesian (`in`) is `id` here.
      var code = String(language.prefix { $0 != "-" && $0 != "_" }).lowercased()
      if code == "in" { code = "id" }
      if let words = table[code] { return words }
    }
    return table["en"]!
  }

  /** "Incoming call", or "Incoming video call". */
  static func incoming(video: Bool) -> String {
    video ? words.incomingVideo : words.incoming
  }

  /** "A call is on", or "A video call is on": the text of a call on in a group. */
  static func groupOn(video: Bool) -> String {
    video ? words.groupVideoOn : words.groupOn
  }
}
