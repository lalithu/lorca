import XCTest
import AppKit
@testable import LorcaMac
@MainActor final class Chunk2Tests:XCTestCase {
 func testThinkingDisclosureAutoCollapseAndManualReopen() {
  var state=ThinkingDisclosure();state.appear(active:true);XCTAssertTrue(state.expanded)
  state.update(active:false);XCTAssertFalse(state.expanded)
  state.toggle();XCTAssertTrue(state.expanded);state.update(active:false);XCTAssertTrue(state.expanded)
  state.toggle();XCTAssertFalse(state.expanded)
  var manual=ThinkingDisclosure();manual.appear(active:true);manual.toggle();manual.update(active:true);XCTAssertFalse(manual.expanded)
 }
 func testReasoningOrderingSeparationAndLifecycle() {
  let b=AgentThinkingBlock(id:"r",state:"complete",text:"actual fixture reasoning",afterSequence:1,textOffset:0,truncated:false)
  let events=[AgentEvent(id:"r:2",sequence:2,taskId:"r",kind:"tool_requested",data:.object(["callId":.string("t"),"tool":.string("read_file"),"presentation":.object(["textBytes":.number(0)])]))]
  let timeline=AgentTimelineItem.make(text:"Final answer",events:events,activities:AgentActivity.make(from:events),thinking:[b]);
  XCTAssertEqual(timeline.first?.content,.thinking(b));XCTAssertEqual(timeline.last?.content,.text("Final answer"));XCTAssertFalse(b.active)
  var off=b;off.state="off";XCTAssertFalse(AgentTimelineItem.make(text:"Answer",events:[],activities:[],thinking:[off]).contains {if case .thinking = $0.content {return true};return false})
 }
 func testSecretRedactionDoesNotChangeOriginal() {
  let raw=AgentJSON.object(["api_key":.string("secret"),"taskId":.string("internal"),"exitCode":.number(1),"nested":.object(["Authorization":.string("bearer")])]);let redacted=raw.redacted
  XCTAssertEqual(redacted["api_key"].string,"[redacted]");XCTAssertEqual(redacted["nested"]["Authorization"].string,"[redacted]");XCTAssertEqual(raw["api_key"].string,"secret");XCTAssertEqual(redacted["exitCode"].int,1)
 }
 func testNativeClipboardAndCodeBlockCopies() {
  let board=NSPasteboard.withUniqueName();defer{board.releaseGlobally()}
  let code="let whale = \"🫍\"";let source="Text\n```swift\n\(code)\n```";XCTAssertEqual(MarkdownText.codeBlocks(source),[code]);NativeClipboard.copy(code,to:board);XCTAssertEqual(board.string(forType:.string),code)
 }
 func testLargeOutputPreviewRetainsFullRecoverableText() {
  let text=String(repeating:"log 🫍\n",count:20000);XCTAssertEqual(AgentOutputView.preview(text).count,12000);XCTAssertTrue(text.hasPrefix(AgentOutputView.preview(text)));XCTAssertGreaterThan(text.count,12000)
 }
 func testSupersededApprovalAndCommandStatesAreNotSuccess() {
  let e:[AgentEvent]=[
   AgentEvent(id:"1",sequence:1,taskId:"r",kind:"tool_requested",data:.object(["callId":.string("t"),"tool":.string("exec_command"),"arguments":.object(["executable":.string("cargo"),"arguments":.array([.string("test")])])])),
   AgentEvent(id:"2",sequence:2,taskId:"r",kind:"task_revision_requested",data:.object([:])),
  ];let activities=AgentActivity.make(from:e);XCTAssertEqual(activities[0].statusLabel,"Superseded");XCTAssertTrue(AgentActivity.summary(activities).contains("superseded"));XCTAssertFalse(AgentActivity.summary(activities).contains("Ran tests"))
  var superseded=AgentActivity(id:"old",tool:"apply_patch");superseded.completed=true;superseded.superseded=true;superseded.arguments = .object(["path":.string("old.txt")])
  var edited=AgentActivity(id:"new",tool:"apply_patch");edited.completed=true;edited.arguments = .object(["path":.string("new.txt")]);XCTAssertTrue(AgentActivity.summary([superseded,edited]).contains("Edited 1 file"))
 }
 func testFocusedLargeDiffPerformanceAndMultipleFiles() {
  let before=(0..<10000).map {"line \($0)"};var after=before;after[5000]="changed";
  let patch="--- a.txt\n+++ a.txt\n@@ -1,10000 +1,10000 @@\n"+before.map{"-"+$0+"\n"}.joined()+after.map{"+"+$0+"\n"}.joined();let t=Date();let parsed=UnifiedDiff(patch);let rows=parsed.rows();let elapsed=Date().timeIntervalSince(t);print("CHUNK2_DIFF_10000_LINES_SECONDS \(elapsed)");XCTAssertLessThan(rows.count,20);XCTAssertEqual(parsed.lines.filter{$0.kind == .addition}.count,1);XCTAssertEqual(parsed.lines.filter{$0.kind == .removal}.count,1)
  let multi=UnifiedDiff("--- /dev/null\n+++ new.txt\n@@ -0,0 +1 @@\n+new\n--- old.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\n");XCTAssertTrue(multi.lines.contains{$0.text=="+++ new.txt"});XCTAssertTrue(multi.lines.contains{$0.text=="+++ /dev/null"})
 }
 func testThinkingChoicePersistsWithoutMutatingActiveTask() async throws {
  let defaults=UserDefaults(suiteName:"lorca.chunk2.\(UUID().uuidString)")!;let backend=AgentMockBackend();let store=ChatStore(backend:backend,defaults:defaults);await store.refreshConnection();store.thinkingEffort = .nominal;store.draft="hello";store.send();try await Task.sleep(for:.milliseconds(100));let id=store.agent.active?.id;store.thinkingEffort = .off;XCTAssertEqual(store.agent.active?.id,id);XCTAssertEqual(defaults.string(forKey:"thinkingEffort"),"off");XCTAssertEqual(backend.requests.first(where:{$0["op"] as? String == "agent_start"})?["effort"] as? String,"nominal");_ = await store.modelSession.close(unload:false)
 }
}
