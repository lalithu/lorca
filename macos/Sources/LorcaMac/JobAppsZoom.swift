import SwiftUI

private struct JobAppsZoomKey: EnvironmentKey { static let defaultValue: CGFloat = 0 }
extension EnvironmentValues {
    var jobAppsZoom: CGFloat {
        get { self[JobAppsZoomKey.self] }
        set { self[JobAppsZoomKey.self] = newValue }
    }
}
