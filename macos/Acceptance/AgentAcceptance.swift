import AppKit
import SwiftUI
import Foundation
enum LorcaAsset {
 static let emoji = (try! String(contentsOfFile:ProcessInfo.processInfo.environment["LORCA_SOURCE_ROOT"]!+"/macos/Sources/LorcaMac/Resources/orca.txt",encoding:.utf8)).trimmingCharacters(in:.whitespacesAndNewlines)
}
@main @MainActor struct NativeAcceptance {
 static func editors(_ view:NSView)->[ChatInput.Editor] { (view as? ChatInput.Editor).map{[$0]} ?? view.subviews.flatMap{editors($0)} }
 static func main() async {
  setbuf(stdout, nil)
  let env=ProcessInfo.processInfo.environment
  let root=URL(fileURLWithPath:env["LORCA_ACCEPTANCE_ROOT"]!)
  let project=root.appendingPathComponent("project")
  let model=env["LORCA_ACCEPTANCE_MODEL"] ?? "qwen3.5:9b-q4_K_M"
  _=NSApplication.shared
  let defaults=UserDefaults(suiteName:"lorca.acceptance.\(UUID().uuidString)")!
  defaults.set(model,forKey:"model")
  let staleRoot=root.appendingPathComponent("stale-backend")
  try! FileManager.default.createDirectory(at:staleRoot.appendingPathComponent("runtime"),withIntermediateDirectories:true)
  var staleEndpoint=(try! JSONSerialization.jsonObject(with:Data(contentsOf:BackendClient.defaultRoot.appendingPathComponent("runtime/endpoint.json")))) as! [String:Any]
  staleEndpoint["protocol"]=2
  try! JSONSerialization.data(withJSONObject:staleEndpoint).write(to:staleRoot.appendingPathComponent("runtime/endpoint.json"))
  do { _=try await BackendClient(root:staleRoot).connect(model:model);print("NATIVE_STALE_BACKEND_ACCEPTED");exit(8) }
  catch {guard error.localizedDescription.contains("Incompatible lorca backend") else {print(error);exit(9)}}
  print("NATIVE_STALE_BACKEND_REJECTED")
  let store=ChatStore(defaults:defaults)
  let windows=WindowManager(store:store)
  windows.open()
  NSApp.setActivationPolicy(.regular)
  NSApp.activate(ignoringOtherApps:true)
  await store.refreshConnection()
  print("NATIVE_READY model=\(model) ready=\(store.ready) connected=\(store.isConnected)")
  guard store.ready && store.isConnected else {print("NATIVE_BLOCKER \(store.error ?? store.modelSession.error ?? "unknown")");exit(1)}
  store.agent.workspaces[store.library.selectedID]=project
  windows.toggleMiniExpansion()
  try? await Task.sleep(for:.milliseconds(200))
  guard NSApp.windows.contains(where:{$0.title == "lorca Mini Chat" && $0.isVisible}) else {print("NATIVE_MINI_MISSING");exit(4)}
  windows.toggleMiniExpansion()
  for status in [AgentTaskSnapshot.Status.incomplete,.denied] {guard !status.active && status != .completed else{exit(5)}}
  var report:[[String:Any]]=[]
  var acceptedAll=true
  let prompts = env["LORCA_ACCEPTANCE_TASK"] == "long" ? [
   "Inspect this repository and update README.md to accurately describe the current implementation. Inspect these actual modules: src/agent/execution.rs, src/agent/completion.rs, src/agent/permissions.rs, src/agent/workspace.rs, src/agent/tools.rs, src/agent/command.rs, src/agent/runtime.rs, src/provider/agent.rs, src/core/agent_chats.rs, and src/context.rs. Document the real architecture, safety boundaries and limitations in a concise README, then run cargo test. Use nextOffset for relevant truncated sections. Keep the README under 250 words."
  ] : [
   "Inspect this repository and update README.md to accurately describe the current implementation.",
   "Fix the subtraction bug in src/lib.rs so add(2, 3) returns 5. Run cargo test after applying the fix and report the actual exit code.",
   "What did you change and what was the test result? Answer concisely from this conversation without using tools."
  ]
  for prompt in prompts {
   let previous=store.agent.latest?.id
   let origin=store.library.selectedID
   var switched=false
   store.draft=prompt
   try? await Task.sleep(for:.milliseconds(200))
   guard store.canSend else {print("NATIVE_SEND_BLOCKED \(store.agent.error ?? "unknown")");acceptedAll=false;break}
   guard let editor=NSApp.windows.first(where:{$0.title == "lorca"})?.contentView.flatMap({editors($0).first}) else {print("NATIVE_EDITOR_MISSING");exit(2)}
   let enter=NSEvent.keyEvent(with:.keyDown,location:.zero,modifierFlags:[],timestamp:0,windowNumber:editor.window?.windowNumber ?? 0,context:nil,characters:"\r",charactersIgnoringModifiers:"\r",isARepeat:false,keyCode:36)!
   editor.keyDown(with:enter)
   print("NATIVE_ENTER \(prompt)")
   let start=Date();var approved=Set<String>();var streamed=false;var logged=Set<UInt64>();var settled:AgentTaskSnapshot?
   while Date().timeIntervalSince(start)<900 {
    if let task=store.agent.latest,task.id != previous {
     if !switched && prompt == prompts.first && task.status.active {
      switched=true
      store.newChat()
      for _ in 0..<50 {if store.library.selectedID != origin {break};try? await Task.sleep(for:.milliseconds(20))}
      guard store.library.selectedID != origin,store.agent.active?.conversationId == origin else {print("NATIVE_SWITCH_FAILED");exit(6)}
      store.openConversation(origin)
      for _ in 0..<50 {if store.library.selectedID == origin {break};try? await Task.sleep(for:.milliseconds(20))}
      guard store.library.selectedID == origin else {exit(7)}
      print("NATIVE_MINI_AND_CONVERSATION_SWITCH_PASSED")
     }
     if task.generating == true && !(task.assistantText ?? "").isEmpty {streamed=true}
     for event in task.events where logged.insert(event.sequence).inserted {
      if ["tool_requested","tool_failed","context_compacted","task_continued","finalization_requested"].contains(event.kind){print("EVENT \(event.kind) \(event.data.pretty.prefix(400))")}
     }
     if let approval=task.approval,!approved.contains(approval.id),store.agent.canDecide(task,approval:approval) {
      let action=approval.action;let path=action["path"].string ?? ""
      let allow = approval.tool == "apply_patch" && ["README.md","src/lib.rs"].contains(path)
       || approval.tool == "run_command" && [ ["test"],["test","--quiet"] ].contains(action["arguments"].strings)
        && action["workingDirectory"].string.map { URL(fileURLWithPath:$0).standardizedFileURL.path } == project.standardizedFileURL.path
      approved.insert(approval.id)
      print("NATIVE_APPROVAL \(approval.tool) allow=\(allow) action=\(action.pretty.prefix(350))")
      await store.agent.decide(task:task,approval:approval,decision:allow ? "allow_once":"deny")
     }
     if !task.status.active {settled=task;break}
    }
    try? await Task.sleep(for:.milliseconds(100))
   }
   guard let task=settled else {await store.agent.cancel();print("NATIVE_TIMEOUT");acceptedAll=false;break}
   let encoder=JSONEncoder();let data=try! encoder.encode(task);var entry=(try! JSONSerialization.jsonObject(with:data)) as! [String:Any]
   entry["readmeReadback"]=(try? String(contentsOf:project.appendingPathComponent("README.md"),encoding:.utf8)) ?? "";entry["sourceReadback"]=(try? String(contentsOf:project.appendingPathComponent("src/lib.rs"),encoding:.utf8)) ?? "";entry["prompt"]=prompt;entry["seconds"]=Date().timeIntervalSince(start);entry["streamed"]=streamed;entry["approvedActions"]=approved.count
   report.append(entry)
   let reportData=try! JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]);try! reportData.write(to:root.appendingPathComponent("native-report.json"))
   print("NATIVE_RESULT \(task.status.rawValue) \(task.result ?? "")")
   if task.status != .completed {acceptedAll=false;break}
   if prompt.hasPrefix("What did"), task.events.contains(where:{$0.kind == "tool_requested"}) {
    print("NATIVE_FOLLOWUP_UNEXPECTED_TOOL");acceptedAll=false
   }
   try? await Task.sleep(for:.milliseconds(400))
  }
  let readme=(try? String(contentsOf:project.appendingPathComponent("README.md"),encoding:.utf8)) ?? ""
  print("NATIVE_READBACK README_BYTES=\(readme.utf8.count)")
  let view=NSApp.windows.first{$0.title == "lorca"}?.contentView
  if let view,let rep=view.bitmapImageRepForCachingDisplay(in:view.bounds) {view.cacheDisplay(in:view.bounds,to:rep);if let data=rep.representation(using:.png,properties:[:]){try? data.write(to:root.appendingPathComponent("native-final.png"))}}
  _=await store.modelSession.close(unload:true)
  print(acceptedAll ? "NATIVE_ACCEPTANCE_PASSED":"NATIVE_ACCEPTANCE_FAILED")
  exit(acceptedAll ? 0:3)
 }
}
