import Darwin
import SwiftUI

// Native VM counters and the OS pressure state. No subprocesses or shared-store
// invalidations; this sidebar view samples at 0.5 seconds and retains 480 points.
struct MemoryMetrics {
    let physical: Double
    let used: Double
    let app: Double
    let wired: Double
    let compressed: Double
    let cached: Double
    let swap: Double?
    var fraction: CGFloat { CGFloat(min(1, max(0, used / max(1, physical)))) }
    private static let host = mach_host_self()

    static func read() -> Self? {
        var vm = vm_statistics64_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<vm_statistics64_data_t>.size / MemoryLayout<integer_t>.size)
        let status = withUnsafeMutablePointer(to: &vm) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                host_statistics64(host, HOST_VM_INFO64, $0, &count)
            }
        }
        guard status == KERN_SUCCESS else { return nil }
        let page = Double(vm_kernel_page_size)
        let app = max(0, Double(vm.internal_page_count) - Double(vm.purgeable_count)) * page
        let wired = Double(vm.wire_count) * page
        let compressed = Double(vm.compressor_page_count) * page
        let cached = (Double(vm.external_page_count) + Double(vm.purgeable_count)) * page
        var swap = xsw_usage()
        var size = MemoryLayout.size(ofValue: swap)
        let swapOK = sysctlbyname("vm.swapusage", &swap, &size, nil, 0) == 0
        return Self(physical: Double(ProcessInfo.processInfo.physicalMemory),
            used: app + wired + compressed, app: app, wired: wired,
            compressed: compressed, cached: cached, swap: swapOK ? Double(swap.xsu_used) : nil)
    }
    static func gigabytes(_ bytes: Double) -> String { String(format: "%.2f GB", bytes / 1_073_741_824) }
}

struct MemoryPressureView: View {
    private enum Pressure: Int {
        case normal = 1, warning = 2, critical = 4
        var label: String { switch self { case .normal: "Normal"; case .warning: "Elevated"; case .critical: "High" } }
        var color: Color { switch self { case .normal: .green; case .warning: .yellow; case .critical: .red } }
        static func read() -> Self? {
            var level: Int32 = 0
            var bytes = MemoryLayout.size(ofValue: level)
            guard sysctlbyname("kern.memorystatus_vm_pressure_level", &level, &bytes, nil, 0) == 0 else { return nil }
            return Self(rawValue: Int(level))
        }
    }
    private struct Sample { let metrics: MemoryMetrics; let date: Date }
    @State private var samples: [Sample] = []
    @State private var available = true
    @State private var pressure: Pressure?
    private let capacity = 480
    private var current: Pressure? { available ? pressure : nil }
    private var tint: Color { current?.color ?? .secondary }

    private func stat(_ label: String, _ bytes: Double?) -> some View {
        HStack(spacing: 4) {
            Text(label).foregroundStyle(.secondary)
            Spacer(minLength: 0)
            Text(bytes.map(MemoryMetrics.gigabytes) ?? "—").monospacedDigit()
        }.font(.system(size: 10))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                Text("Memory pressure")
                Spacer(minLength: 4)
                Text(current?.label ?? "Unavailable").foregroundStyle(current?.color ?? .secondary)
            }.font(.system(size: 10)).foregroundStyle(.secondary)
            HStack {
                Text("RAM usage")
                Spacer()
                if let metrics = samples.last?.metrics { Text(String(format: "%.0f%%", Double(metrics.fraction) * 100)).monospacedDigit() }
            }.font(.system(size: 9)).foregroundStyle(.secondary)
            Canvas { context, size in
                guard let last = samples.last else { return }
                let step = size.width / CGFloat(capacity - 1)
                let firstX = size.width - CGFloat(samples.count - 1) * step
                var line = Path()
                for (index, sample) in samples.enumerated() {
                    let point = CGPoint(x: firstX + CGFloat(index) * step, y: size.height * (1 - sample.metrics.fraction))
                    if index == 0 { line.move(to: point) } else { line.addLine(to: point) }
                }
                var area = line
                area.addLine(to: CGPoint(x: size.width, y: size.height))
                area.addLine(to: CGPoint(x: firstX, y: size.height))
                area.closeSubpath()
                context.fill(area, with: .color(tint.opacity(0.17)))
                context.stroke(line, with: .color(tint.opacity(0.8)), lineWidth: 1)
                let dot = CGRect(x: size.width - 3, y: size.height * (1 - last.metrics.fraction) - 1.5, width: 3, height: 3)
                context.fill(Path(ellipseIn: dot), with: .color(tint))
            }
            .frame(height: 37)
            .background(.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 4))
            .clipShape(RoundedRectangle(cornerRadius: 4))
            .overlay(alignment: .bottom) { Rectangle().fill(.secondary.opacity(0.15)).frame(height: 0.5) }
            .opacity(available ? 1 : 0.4)
            if available, let metrics = samples.last?.metrics {
                VStack(spacing: 4) {
                    stat("Physical", metrics.physical)
                    stat("Used", metrics.used)
                    stat("App", metrics.app)
                    stat("Wired", metrics.wired)
                    stat("Compressed", metrics.compressed)
                    stat("Cached files", metrics.cached)
                    stat("Swap", metrics.swap)
                }.padding(.top, 3)
            }
        }
        .padding(.horizontal, 9)
        .padding(.bottom, 17)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Memory pressure: \(current?.label ?? "Unavailable")")
        .help("Live macOS VM counters, sampled every 0.5 seconds. The graph shows RAM usage over four minutes; its color is the OS memory-pressure state. Used = app + wired + compressed. Values may differ from Activity Monitor due to sampling and accounting; this is not its pressure-curve algorithm.")
        .task {
            while !Task.isCancelled {
                if let metrics = MemoryMetrics.read() {
                    available = true
                    pressure = Pressure.read()
                    if let previous = samples.last, Date().timeIntervalSince(previous.date) > 15 { samples.removeAll() }
                    samples.append(Sample(metrics: metrics, date: Date()))
                    if samples.count > capacity { samples.removeFirst(samples.count - capacity) }
                } else { available = false }
                do { try await Task.sleep(for: .milliseconds(500)) } catch { break }
            }
        }
    }
}
