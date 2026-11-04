import AppKit
import SwiftUI

@MainActor enum Chunk2Acceptance {
 static func capture(_ view:NSView,_ name:String,_ root:URL) {
  view.layoutSubtreeIfNeeded()
  if let rep=view.bitmapImageRepForCachingDisplay(in:view.bounds) {
   view.cacheDisplay(in:view.bounds,to:rep)
   if let data=rep.representation(using:.png,properties:[:]) {try? data.write(to:root.appendingPathComponent(name+".png"));print("UI_CAPTURE \(name)")}
  }
 }
 static func run(store:ChatStore,windows:WindowManager,root:URL) async {
  guard let dashboard=NSApp.windows.first(where:{$0.title=="lorca"}),let content=dashboard.contentView else {exit(31)}
  dashboard.setContentSize(NSSize(width:1100,height:850));store.appearance = .dark
  var report:[[String:Any]]=[];var liveThinking:[AgentThinkingBlock]=[]
  func actual(_ prompt:String,_ effort:ThinkingEffort,_ name:String,revise:Bool=false) async -> AgentTaskSnapshot? {
   let previous=store.agent.latest?.id;store.thinkingEffort=effort;store.draft=prompt
   guard store.canSend else {print("UI_SEND_BLOCKED");return nil}
   store.send();let start=Date();var captured=false;var capturedReasoning=false;var approved=Set<String>();var corrected=false;var settingChanged=false
   while Date().timeIntervalSince(start)<240 {
    if let task=store.agent.latest,task.id != previous {
     if task.generating == true && !(task.assistantText ?? "").isEmpty && !captured {capture(content,name+"-streaming",root);captured=true}
     if let blocks=task.thinking,blocks.contains(where:{$0.active && !$0.text.isEmpty}),!capturedReasoning {
      capturedReasoning=true;capture(content,"thinking-on-live",root)
      // The setting changes for the NEXT task, leaving this request untouched.
      store.thinkingEffort = .off;settingChanged=true
     }
     if let approval=task.approval,!approved.contains(approval.id) {
      capture(content,corrected ? "revised-approval-live":"pending-approval-live",root)
      if revise && !corrected {
       store.draft="What are you doing?";store.send()
       for _ in 0..<100 {if !store.agent.submitting {break};try? await Task.sleep(for:.milliseconds(20))}
       store.draft="Correction: change README.md title to # Corrected UI acceptance. Preserve the rest. Edit README.md only. Do not run commands.";store.send();corrected=true
       for _ in 0..<100 {if !store.agent.submitting {break};try? await Task.sleep(for:.milliseconds(20))}
       guard !store.agent.canDecide(task,approval:approval) else {print("UI_STALE_APPROVAL_ACTIONABLE");return nil}
       capture(content,"correction-live",root)
      } else if store.agent.canDecide(task,approval:approval) {
       let safe=approval.tool=="apply_patch" && approval.action["path"].string=="README.md"
       approved.insert(approval.id);await store.agent.decide(task:task,approval:approval,decision:safe ? "allow_once":"deny")
      }
     }
     if !task.status.active {
      let blocks=task.thinking ?? []
      report.append(["name":name,"status":task.status.rawValue,"seconds":Date().timeIntervalSince(start),"thinkingBytes":blocks.reduce(0){$0+$1.text.utf8.count},"thinkingStates":blocks.map(\.state),"generatedTokens":task.usage?.generatedTokens.map {NSNumber(value:$0)} ?? NSNull(),"modeChangeDuringRequest":settingChanged,"correction":corrected])
      try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:root.appendingPathComponent("ui-report.json"))
      if effort == .off && blocks.contains(where:{!$0.text.isEmpty}) {print("UI_OFF_LEAKED_REASONING");return nil}
      if effort != .off {liveThinking=blocks.filter{!$0.text.isEmpty};if liveThinking.isEmpty {print("UI_ON_NO_REASONING");return nil}
       guard blocks.contains(where:{$0.stage == .interpretation}),blocks.contains(where:{$0.stage == .execution}) else {print("UI_INFERENCE_STAGE_MISSING");return nil}}
      capture(content,name+"-final",root)
      print("UI_LIVE \(name) status=\(task.status.rawValue) seconds=\(Date().timeIntervalSince(start)) reasoningBytes=\(blocks.reduce(0){$0+$1.text.utf8.count})")
      return task.status == .completed ? task:nil
     }
    }
    try? await Task.sleep(for:.milliseconds(50))
   }
   await store.agent.cancel();print("UI_LIVE_TIMEOUT");return nil
  }
  guard await actual("Explain in five short paragraphs how to safely inspect, edit, test and review a small Rust change. No tools.",.off,"thinking-off") != nil else {exit(32)}
  guard await actual("Read README.md and src/lib.rs. Summarize what each file contains in three sentences. Do not edit files or run commands.",.nominal,"thinking-on") != nil else {exit(33)}
  // Manual UI disclosure follows the cached task across actual conversation navigation.
  if let task=store.agent.latest,let block=task.thinking?.first(where:{!$0.text.isEmpty}) {
   store.agent.setThinkingExpanded(true,taskID:task.id,blockID:block.id)
   let original=store.library.selectedID;store.newChat()
   for _ in 0..<100 {if store.library.selectedID != original {break};try? await Task.sleep(for:.milliseconds(20))}
   store.openConversation(original)
   for _ in 0..<100 {if store.library.selectedID == original {break};try? await Task.sleep(for:.milliseconds(20))}
   guard store.agent.thinkingExpanded(taskID:task.id,block:block) else {print("UI_DISCLOSURE_LOST");exit(37)}
   try? await Task.sleep(for:.milliseconds(250));capture(content,"thinking-reopened-after-chat-switch",root)
   store.agent.setThinkingExpanded(false,taskID:task.id,blockID:block.id)
  }
  // Independent write scenario: keep the preceding explicit read-only task intact.
  let previousConversation=store.library.selectedID;store.newChat()
  for _ in 0..<100 {if store.library.selectedID != previousConversation {break};try? await Task.sleep(for:.milliseconds(20))}
  store.agent.workspaces[store.library.selectedID]=root.appendingPathComponent("project")
  guard let edited=await actual("Change the README.md title to # Native UI acceptance. Preserve the rest of the file. Edit README.md only. Do not run commands.",.off,"editing",revise:true) else {exit(34)}
  guard (try? String(contentsOf:root.appendingPathComponent("project/README.md"),encoding:.utf8)) == "# Corrected UI acceptance\n\nThis repository is an unfinished Python web server.\n" else {print("UI_CORRECTION_READBACK_FAILED");exit(36)}
  let taskID=edited.id
  let visual=NSWindow(contentRect:NSRect(x:0,y:0,width:1100,height:900),styleMask:[.titled,.closable],backing:.buffered,defer:false)
  visual.title="Lorca · deterministic visual fixtures";visual.isReleasedWhenClosed=false
  for appearance in [NSAppearance.Name.darkAqua,.aqua] {
   visual.appearance=NSAppearance(named:appearance)
   visual.contentView=NSHostingView(rootView:ScrollView {
    VStack(alignment:.leading,spacing:18) {
     Text("Native components · recorded execution and deterministic rendering fixtures").font(.headline)
     Text("Actual model thinking replay; intentionally captured for visual acceptance only.").font(.caption).foregroundStyle(.secondary)
     if let block=liveThinking.first {ThinkingPanel(block:block,initiallyExpanded:false);ThinkingPanel(block:block,initiallyExpanded:true)}
     AgentActivityView(store:store.agent,taskID:taskID,textSize:13,expandTools:true)
     Text("Deterministic long-output fixture").font(.headline)
     AgentOutputView(text:(0..<3000).map{"test row \($0): passed"}.joined(separator:"\n"))
     MarkdownText(text:"## Select and copy\nNormal **Markdown**, `inline code`, and a code block.\n\n```rust\nfn main() { println!(\"Hello\"); }\n```\n\n| Test | Result |\n| --- | --- |\n| Native | Passed |",size:14)
    }.padding(20).frame(width:1000,alignment:.leading)
   }.frame(width:1100,height:900).background(Color(nsColor:.windowBackgroundColor)).preferredColorScheme(appearance == .darkAqua ? .dark:.light))
   visual.center();visual.makeKeyAndOrderFront(nil);try? await Task.sleep(for:.milliseconds(500))
   if let v=visual.contentView {capture(v,appearance == .darkAqua ? "components-dark":"components-light",root)}
  }
  visual.close()
  func scene<V:View>(_ name:String,_ view:V) async {
   let window=NSWindow(contentRect:NSRect(x:0,y:0,width:1000,height:800),styleMask:[.titled],backing:.buffered,defer:false);window.isReleasedWhenClosed=false;window.appearance=NSAppearance(named:.darkAqua)
   window.contentView=NSHostingView(rootView:ScrollView {VStack(alignment:.leading,spacing:15) {Text(name).font(.headline);view}.padding(20).frame(maxWidth:.infinity,alignment:.leading)}.frame(width:1000,height:800).background(Color(nsColor:.windowBackgroundColor)).preferredColorScheme(.dark));window.makeKeyAndOrderFront(nil);try? await Task.sleep(for:.milliseconds(300));if let v=window.contentView {capture(v,name,root)};window.close()
  }
  if let block=liveThinking.first {await scene("thinking-expanded-replay",ThinkingPanel(block:block,initiallyExpanded:true));await scene("thinking-collapsed-replay",ThinkingPanel(block:block,initiallyExpanded:false))}
  await scene("tool-cards-expanded",AgentActivityView(store:store.agent,taskID:taskID,textSize:13,expandTools:true))
  await scene("tool-cards-collapsed",AgentActivityView(store:store.agent,taskID:taskID,textSize:13))
  let lines=(0..<1000).map {"line \($0)"};var changed=lines;changed[500]="changed line"
  let patch="--- large.txt\n+++ large.txt\n@@ -1,1000 +1,1000 @@\n"+lines.map{"-"+$0+"\n"}.joined()+changed.map{"+"+$0+"\n"}.joined()
  await scene("focused-large-diff-fixture",AgentDiffView(diff:patch))
  await scene("long-terminal-fixture",AgentOutputView(text:(0..<3000).map {"Test \($0) passed"}.joined(separator:"\n")))
  dashboard.makeKeyAndOrderFront(nil)
  store.appearance = .light;try? await Task.sleep(for:.milliseconds(300));capture(content,"conversation-light",root)
  store.appearance = .dark;windows.toggleMiniExpansion();try? await Task.sleep(for:.milliseconds(300))
  if let mini=NSApp.windows.first(where:{$0.title=="lorca Mini Chat"}),let v=mini.contentView {capture(v,"mini",root)}else{exit(35)}
  windows.toggleMiniExpansion()
  // Clarification and failure states are explicit deterministic fixtures, never live claims.
  let fixtureID="ui-fixture"
  func event(_ seq:UInt64,_ kind:String,_ data:[String:AgentJSON])->AgentEvent {AgentEvent(id:"fixture:\(seq)",sequence:seq,taskId:fixtureID,kind:kind,data:.object(data))}
  let ev=[event(1,"tool_requested",["callId":.string("question"),"tool":.string("request_user_input")]),event(2,"tool_completed",["callId":.string("question"),"result":.object(["needsUserInput":.bool(true),"question":.string("Which fixture should I inspect?"),"options":.array([.string("README.md"),.string("src/lib.rs")])])]),event(3,"tool_requested",["callId":.string("test"),"tool":.string("exec_command"),"arguments":.object(["executable":.string("cargo"),"arguments":.array([.string("test")])])]),event(4,"tool_failed",["callId":.string("test"),"result":.object(["ok":.bool(false),"exitCode":.number(1),"output":.string("Fixture failure: expected 5, got -1")])])]
  store.agent.apply(AgentTaskSnapshot(id:fixtureID,owner:"fixture",conversationId:store.library.selectedID,requestId:nil,task:"Deterministic clarification fixture",workspace:"",status:.waitingForInput,events:ev,approval:nil,result:"Which fixture should I inspect?"))
  let qwindow=NSWindow(contentRect:NSRect(x:0,y:0,width:1000,height:800),styleMask:[.titled],backing:.buffered,defer:false);qwindow.isReleasedWhenClosed=false
  qwindow.contentView=NSHostingView(rootView:VStack(alignment:.leading) {Text("Deterministic clarification / failure fixture").font(.headline);AgentActivityView(store:store.agent,taskID:fixtureID,onChoose:{_ in},expandTools:true)}.padding(20).frame(width:1000,height:800,alignment:.topLeading).background(Color(nsColor:.windowBackgroundColor)).preferredColorScheme(.dark));qwindow.makeKeyAndOrderFront(nil);try? await Task.sleep(for:.milliseconds(300));if let v=qwindow.contentView {capture(v,"clarification-failure-fixture",root)};qwindow.close()
  try? JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:root.appendingPathComponent("ui-report.json"))
  _=await store.modelSession.close(unload:false);print("CHUNK2_NATIVE_ACCEPTANCE_PASSED");exit(0)
 }
}
