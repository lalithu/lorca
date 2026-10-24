import XCTest
@testable import LorcaMac

@MainActor
final class ModelSessionTests: XCTestCase {
    func testLifecycleStateComesFromCoreAndActionsCarryConsent() async throws {
        let backend = MockBackend()
        let session = ModelSession()
        session.apply(try backend.snapshot().sessions[0])
        XCTAssertEqual(session.activeModel, backend.model)
        XCTAssertEqual(session.modelsToRelease, [backend.model])
        var approved: Bool?
        session.approvePreviousUnload = { _ in true }
        session.prepareAction = { model, consent in approved = consent; return model == "other" }
        let ready = await session.prepare("other").value
        XCTAssertTrue(ready)
        XCTAssertEqual(approved, true)
        session.closeAction = { unload in unload ? ["unconfirmed"] : [] }
        let failures = await session.close(unload: true)
        XCTAssertEqual(failures, ["unconfirmed"])
        XCTAssertTrue(session.isClosing)
    }
    func testDecliningUnloadIsPassedToRust() async {
        let session = ModelSession()
        var consent: Bool?
        session.prepareAction = { _, approved in consent = approved; return true }
        _ = await session.prepare("orca").value
        XCTAssertEqual(consent, false)
    }
}
