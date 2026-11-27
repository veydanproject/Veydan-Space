// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

// «Поделиться» into Veydan from another app on the iPhone
// (internal/messenger-wire.md §5): the share extension of the app. It is a
// process of its own, without the app: what the system hands over (a text,
// a link, photos, videos, files) is copied into the container of the app
// group, the Rust of crates/share-ext opens the app's data there, lists
// the chats, and sends into the chosen one by itself; the app, closed or
// in the background, sees the messages when it comes back. The lock of the
// app, when set, is asked for here (its PIN or password).

import UIKit
import UniformTypeIdentifiers

/// What the system shows (NSExtensionPrincipalClass): the screen inside a navigation bar of its own.
@objc(ShareNavigationController)
final class ShareNavigationController: UINavigationController {
  init() {
    super.init(rootViewController: ShareViewController())
  }

  required init?(coder: NSCoder) {
    super.init(rootViewController: ShareViewController())
  }
}

final class ShareViewController: UIViewController, UITableViewDataSource, UITableViewDelegate {
  /// The answers and the stages of crates/share-ext (veydan_share.h), as numbers: an anonymous C enum has no type in Swift.
  private static let ok: Int32 = 0
  private static let locked: Int32 = 1
  private static let noIdentity: Int32 = 2
  private static let stageUploading: Int32 = 2
  private static let stageSending: Int32 = 3
  private static let stageDone: Int32 = 4

  private struct Chat {
    let id: String
    let title: String
    let kind: String
  }

  private let table = UITableView(frame: .zero, style: .insetGrouped)
  private let status = UILabel()
  private let progress = UIProgressView(progressViewStyle: .default)
  private let whatLabel = UILabel()
  private var sendButton: UIBarButtonItem!

  private var chats: [Chat] = []
  private var chosen: Chat?
  private var files: [URL] = []
  private var text: String?
  private var inbox: URL?
  private var busy = false
  private let work = DispatchQueue(label: "net.veydan.share", qos: .userInitiated)

  // MARK: - The screen

  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .systemGroupedBackground
    title = NSLocalizedString("share_title", comment: "")
    navigationItem.leftBarButtonItem = UIBarButtonItem(title: NSLocalizedString("share_cancel", comment: ""), style: .plain, target: self, action: #selector(cancel))
    sendButton = UIBarButtonItem(title: NSLocalizedString("share_send", comment: ""), style: .done, target: self, action: #selector(send))
    sendButton.isEnabled = false
    navigationItem.rightBarButtonItem = sendButton

    whatLabel.font = .preferredFont(forTextStyle: .subheadline)
    whatLabel.textColor = .secondaryLabel
    whatLabel.numberOfLines = 3
    status.font = .preferredFont(forTextStyle: .footnote)
    status.textColor = .secondaryLabel
    status.textAlignment = .center
    progress.isHidden = true

    let head = UIStackView(arrangedSubviews: [whatLabel, progress, status])
    head.axis = .vertical
    head.spacing = 8
    head.isLayoutMarginsRelativeArrangement = true
    head.layoutMargins = UIEdgeInsets(top: 12, left: 20, bottom: 8, right: 20)
    head.translatesAutoresizingMaskIntoConstraints = false
    table.translatesAutoresizingMaskIntoConstraints = false
    table.dataSource = self
    table.delegate = self
    table.register(UITableViewCell.self, forCellReuseIdentifier: "chat")
    view.addSubview(head)
    view.addSubview(table)
    NSLayoutConstraint.activate([
      head.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor),
      head.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      head.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      table.topAnchor.constraint(equalTo: head.bottomAnchor),
      table.leadingAnchor.constraint(equalTo: view.leadingAnchor),
      table.trailingAnchor.constraint(equalTo: view.trailingAnchor),
      table.bottomAnchor.constraint(equalTo: view.bottomAnchor),
    ])
    say(NSLocalizedString("share_loading", comment: ""))
    takeItems()
  }

  private func say(_ s: String) {
    status.text = s
  }

  // MARK: - What is shared

  /// The group of the containing app: the extension is `<app>.share`.
  private static var groupId: String {
    let own = Bundle.main.bundleIdentifier ?? "net.veydan.mobile.share"
    let app = own.hasSuffix(".share") ? String(own.dropLast(".share".count)) : own
    return "group.\(app)"
  }

  /// `<group>/veydan/data`, where the app keeps its data (crates/shell/src/ios_group.rs).
  private static var dataDir: URL? {
    FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupId)?
      .appendingPathComponent("veydan", isDirectory: true)
      .appendingPathComponent("data", isDirectory: true)
  }

  /// The copies of the shared files, in a folder of their own under the group: gone when the extension is done.
  private static func newInbox() -> URL? {
    guard let root = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupId) else { return nil }
    let dir = root.appendingPathComponent("veydan", isDirectory: true)
      .appendingPathComponent("share-inbox", isDirectory: true)
      .appendingPathComponent(String(UInt64(Date().timeIntervalSince1970 * 1000), radix: 16), isDirectory: true)
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir
  }

  private func takeItems() {
    guard let inbox = Self.newInbox() else { return fail("no app group") }
    self.inbox = inbox
    let group = DispatchGroup()
    var texts: [String] = []
    var copied: [URL] = []
    let lock = NSLock()
    let items = (extensionContext?.inputItems as? [NSExtensionItem]) ?? []
    for item in items {
      for provider in item.attachments ?? [] {
        if provider.hasItemConformingToTypeIdentifier(UTType.url.identifier) && !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
          group.enter()
          provider.loadItem(forTypeIdentifier: UTType.url.identifier) { value, _ in
            if let url = value as? URL { lock.lock(); texts.append(url.absoluteString); lock.unlock() }
            group.leave()
          }
        } else if provider.hasItemConformingToTypeIdentifier(UTType.plainText.identifier) && !provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) && !provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
          group.enter()
          provider.loadItem(forTypeIdentifier: UTType.plainText.identifier) { value, _ in
            if let s = value as? String { lock.lock(); texts.append(s); lock.unlock() }
            else if let d = value as? Data, let s = String(data: d, encoding: .utf8) { lock.lock(); texts.append(s); lock.unlock() }
            group.leave()
          }
        } else if let type = provider.registeredTypeIdentifiers.first(where: { UTType($0)?.conforms(to: .data) ?? false }) {
          group.enter()
          // The file the provider writes lives only through this closure: copied into the inbox at once.
          provider.loadFileRepresentation(forTypeIdentifier: type) { url, _ in
            if let url = url {
              lock.lock()
              let name = Self.safeName(url.lastPathComponent, index: copied.count)
              let dest = Self.free(inbox.appendingPathComponent(name))
              if (try? FileManager.default.copyItem(at: url, to: dest)) != nil { copied.append(dest) }
              lock.unlock()
            }
            group.leave()
          }
        }
      }
    }
    group.notify(queue: .main) { [weak self] in
      guard let self = self else { return }
      self.files = copied.sorted { $0.lastPathComponent < $1.lastPathComponent }
      let text = texts.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
      self.text = text.isEmpty ? nil : text
      if self.files.isEmpty && self.text == nil {
        return self.fail(NSLocalizedString("share_nothing", comment: ""))
      }
      self.showWhat()
      self.open(secret: nil)
    }
  }

  /// A name the inbox keeps a file under: the last segment, no hidden file, `file-<n>` when nothing usable came.
  private static func safeName(_ raw: String, index: Int) -> String {
    let cleaned = raw.components(separatedBy: CharacterSet.controlCharacters).joined().trimmingCharacters(in: .whitespaces)
    if cleaned.isEmpty || cleaned.hasPrefix(".") || cleaned == ".." { return "file-\(index + 1)" }
    return String(cleaned.prefix(255))
  }

  private static func free(_ url: URL) -> URL {
    var candidate = url
    var n = 2
    while FileManager.default.fileExists(atPath: candidate.path) {
      let ext = url.pathExtension
      let stem = url.deletingPathExtension().lastPathComponent
      candidate = url.deletingLastPathComponent().appendingPathComponent(ext.isEmpty ? "\(stem) (\(n))" : "\(stem) (\(n)).\(ext)")
      n += 1
    }
    return candidate
  }

  private func showWhat() {
    var parts: [String] = []
    if files.count == 1 { parts.append(NSLocalizedString("share_file", comment: "") + ": " + files[0].lastPathComponent) }
    else if files.count > 1 { parts.append(String(format: NSLocalizedString("share_files", comment: ""), files.count) + ": " + files.prefix(3).map { $0.lastPathComponent }.joined(separator: ", ") + (files.count > 3 ? " +\(files.count - 3)" : "")) }
    if let text = text { parts.append(text) }
    whatLabel.text = parts.joined(separator: "\n")
  }

  // MARK: - The messenger

  private func open(secret: String?) {
    guard let dataDir = Self.dataDir else { return fail("no app group") }
    busy = true
    work.async { [weak self] in
      let code = veydan_share_open(dataDir.path, secret)
      var rows: [Chat] = []
      if code == Self.ok, let raw = veydan_share_chats() {
        defer { veydan_share_free(raw) }
        let json = String(cString: raw)
        if let list = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]] {
          rows = list.compactMap { r in
            guard let id = r["id"] as? String, let title = r["title"] as? String, let kind = r["kind"] as? String else { return nil }
            return Chat(id: id, title: title, kind: kind)
          }
        }
      }
      let why = Self.lastError()
      DispatchQueue.main.async { [weak self] in
        guard let self = self else { return }
        self.busy = false
        switch code {
        case Self.ok:
          self.chats = rows
          self.say(rows.isEmpty ? NSLocalizedString("share_no_chats", comment: "") : "")
          self.table.reloadData()
        case Self.locked:
          self.askSecret(wrong: secret != nil)
        case Self.noIdentity:
          self.fail(NSLocalizedString("share_no_identity", comment: ""))
        default:
          self.fail(why ?? NSLocalizedString("share_failed", comment: ""))
        }
      }
    }
  }

  private static func lastError() -> String? {
    guard let raw = veydan_share_error() else { return nil }
    defer { veydan_share_free(raw) }
    return String(cString: raw)
  }

  private func askSecret(wrong: Bool) {
    let alert = UIAlertController(title: NSLocalizedString("share_locked_title", comment: ""), message: NSLocalizedString("share_locked_text", comment: ""), preferredStyle: .alert)
    alert.addTextField { field in
      field.placeholder = NSLocalizedString("share_locked_field", comment: "")
      field.isSecureTextEntry = true
    }
    alert.addAction(UIAlertAction(title: NSLocalizedString("share_cancel", comment: ""), style: .cancel) { [weak self] _ in self?.cancel() })
    alert.addAction(UIAlertAction(title: NSLocalizedString("share_unlock", comment: ""), style: .default) { [weak self, weak alert] _ in
      self?.open(secret: alert?.textFields?.first?.text ?? "")
    })
    present(alert, animated: true)
  }

  @objc private func send() {
    guard !busy, let chat = chosen else { return }
    busy = true
    sendButton.isEnabled = false
    table.isUserInteractionEnabled = false
    progress.isHidden = false
    progress.progress = 0
    say(NSLocalizedString("share_preparing", comment: ""))
    let paths = files.map { $0.path }
    let pathsJson = (try? JSONSerialization.data(withJSONObject: paths)).flatMap { String(data: $0, encoding: .utf8) } ?? "[]"
    let text = self.text
    let ctx = Unmanaged.passRetained(self).toOpaque()
    work.async {
      let code = veydan_share_send(chat.id, text, pathsJson, { ctx, stage, done, total in
        guard let ctx = ctx else { return }
        let me = Unmanaged<ShareViewController>.fromOpaque(ctx).takeUnretainedValue()
        DispatchQueue.main.async { me.progressed(stage: stage, done: done, total: total) }
      }, ctx)
      let why = Self.lastError()
      Unmanaged<ShareViewController>.fromOpaque(ctx).release()
      DispatchQueue.main.async { [weak self] in
        guard let self = self else { return }
        self.busy = false
        if code == Self.ok {
          self.say(NSLocalizedString("share_done", comment: ""))
          self.finish(cancelled: false)
        } else {
          self.progress.isHidden = true
          self.sendButton.isEnabled = true
          self.table.isUserInteractionEnabled = true
          let alert = UIAlertController(title: NSLocalizedString("share_failed", comment: ""), message: why, preferredStyle: .alert)
          alert.addAction(UIAlertAction(title: "OK", style: .default))
          self.present(alert, animated: true)
        }
      }
    }
  }

  private func progressed(stage: Int32, done: UInt64, total: UInt64) {
    switch stage {
    case Self.stageUploading:
      let pct = total > 0 ? Int(done * 100 / total) : 0
      progress.progress = total > 0 ? Float(done) / Float(total) : 0
      say(String(format: NSLocalizedString("share_uploading", comment: ""), pct))
    case Self.stageSending:
      progress.progress = 1
      say(NSLocalizedString("share_sending", comment: ""))
    case Self.stageDone:
      progress.progress = 1
      say(NSLocalizedString("share_done", comment: ""))
    default:
      say(NSLocalizedString("share_preparing", comment: ""))
    }
  }

  // MARK: - The end

  private func fail(_ why: String) {
    let alert = UIAlertController(title: NSLocalizedString("share_failed", comment: ""), message: why, preferredStyle: .alert)
    alert.addAction(UIAlertAction(title: "OK", style: .default) { [weak self] _ in self?.finish(cancelled: true) })
    present(alert, animated: true)
  }

  @objc private func cancel() {
    finish(cancelled: true)
  }

  private func finish(cancelled: Bool) {
    let inbox = self.inbox
    work.async {
      veydan_share_close()
      if let inbox = inbox { try? FileManager.default.removeItem(at: inbox) }
      DispatchQueue.main.async { [weak self] in
        if cancelled {
          self?.extensionContext?.cancelRequest(withError: NSError(domain: "net.veydan.share", code: 1))
        } else {
          self?.extensionContext?.completeRequest(returningItems: nil)
        }
      }
    }
  }

  // MARK: - The list

  func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { chats.count }

  func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
    let cell = tableView.dequeueReusableCell(withIdentifier: "chat", for: indexPath)
    let chat = chats[indexPath.row]
    cell.textLabel?.text = chat.title
    cell.imageView?.image = UIImage(systemName: chat.kind == "group" ? "person.2" : "person")
    cell.accessoryType = chosen?.id == chat.id ? .checkmark : .none
    return cell
  }

  func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
    chosen = chats[indexPath.row]
    sendButton.isEnabled = !busy
    tableView.reloadData()
  }
}
