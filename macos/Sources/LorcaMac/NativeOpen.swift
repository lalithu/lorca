import AppKit

// Headless entry point used by the Rust open_path tool, after its approval gate.
// Reuse the signed native executable without creating another Lorca window/session.
@MainActor
enum NativeOpen {
    static func run(arguments: [String]) {
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        Task { @MainActor in
            do {
                guard arguments.count == 1 || arguments.count == 2 else {
                    throw NSError(domain: "LorcaOpen", code: 1,
                                  userInfo: [NSLocalizedDescriptionKey: "Expected a path and optional application name."])
                }
                let file = URL(fileURLWithPath: arguments[0])
                let workspace = NSWorkspace.shared
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = true
                configuration.addsToRecentItems = true
                let target: NSRunningApplication
                if arguments.count == 2 {
                    let name = arguments[1]
                    let applicationURL: URL?
                    if name.hasPrefix("/") {
                        applicationURL = URL(fileURLWithPath: name)
                    } else {
                        // Resolve bundle IDs through Launch Services; names through installed handlers,
                        // running apps, then the standard system/user Applications directories.
                        let leaf = name.hasSuffix(".app") ? name : name + ".app"
                        let candidates = workspace.urlsForApplications(toOpen: file)
                            + workspace.runningApplications.compactMap(\.bundleURL)
                            + [URL(fileURLWithPath: "/Applications"),
                               URL(fileURLWithPath: "/System/Applications"),
                               FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications")]
                                .map { $0.appendingPathComponent(leaf) }
                        applicationURL = workspace.urlForApplication(withBundleIdentifier: name)
                            ?? candidates.first { $0.lastPathComponent.caseInsensitiveCompare(leaf) == .orderedSame
                                && FileManager.default.fileExists(atPath: $0.path) }
                    }
                    guard let applicationURL else {
                        throw NSError(domain: "LorcaOpen", code: 2,
                                      userInfo: [NSLocalizedDescriptionKey: "Application not found: \(name)"])
                    }
                    target = try await workspace.open([file], withApplicationAt: applicationURL,
                                                      configuration: configuration)
                } else {
                    target = try await workspace.open(file, configuration: configuration)
                }
                // An existing app may receive the document before activation settles.
                // Request activation once, then observe; never claim foreground from launch alone.
                target.activate(options: [.activateAllWindows])
                for _ in 0..<20 {
                    if workspace.frontmostApplication?.processIdentifier == target.processIdentifier { break }
                    try await Task.sleep(for: .milliseconds(50))
                }
                let foreground = workspace.frontmostApplication?.processIdentifier == target.processIdentifier
                var result: [String: Any] = ["ok": true, "opened": true,
                    "foreground": foreground, "path": file.path,
                    "application": target.localizedName ?? "", "pid": target.processIdentifier]
                if !foreground {
                    result["warning"] = "The document was opened, but macOS did not confirm foreground activation."
                }
                finish(result, status: 0)
            } catch {
                finish(["ok": false, "opened": false, "foreground": false,
                        "error": error.localizedDescription], status: 1)
            }
        }
        app.run()
    }

    private static func finish(_ result: [String: Any], status: Int32) -> Never {
        if let data = try? JSONSerialization.data(withJSONObject: result) {
            FileHandle.standardOutput.write(data)
            FileHandle.standardOutput.write(Data([10]))
        }
        exit(status)
    }
}
