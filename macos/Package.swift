// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "LorcaMac",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "lorca-macos", targets: ["LorcaMac"])],
    targets: [
        .executableTarget(name: "LorcaMac", resources: [.copy("Resources/orca.txt")]),
        .testTarget(name: "LorcaMacTests", dependencies: ["LorcaMac"])
    ]
)
