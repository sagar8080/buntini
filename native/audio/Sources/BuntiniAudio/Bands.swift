// Nine voice bands for the pill: a 1024-point FFT of the last 1024 samples (64 ms at 16 kHz),
// Hann-windowed, its power summed into bands spaced logarithmically from 90 Hz to 7 kHz, in
// decibels mapped to 0…1.
import Accelerate
import Foundation

final class BandAnalyzer {
    static let size = 1024
    static let count = 9
    static let low: Double = 90
    static let high: Double = 7_000
    /// Quieter than this is nothing; louder than `ceiling` is full.
    static let floor: Float = -75
    static let ceiling: Float = -20

    private let setup: FFTSetup
    private let window: [Float]
    private var ring = [Float](repeating: 0, count: size)
    private var filled = 0
    /// Bin ranges, half-open, one per band.
    let ranges: [Range<Int>]

    init(rate: Double = Microphone.rate) {
        setup = vDSP_create_fftsetup(vDSP_Length(10), FFTRadix(kFFTRadix2))!
        window = vDSP.window(ofType: Float.self, usingSequence: .hanningDenormalized, count: Self.size, isHalfWindow: false)
        let ratio = Self.high / Self.low
        var ranges: [Range<Int>] = []
        var previous = Int((Self.low * Double(Self.size) / rate).rounded(.down))
        for band in 1...Self.count {
            let edge = Self.low * pow(ratio, Double(band) / Double(Self.count))
            var bin = Int((edge * Double(Self.size) / rate).rounded())
            bin = max(bin, previous + 1)
            ranges.append(previous..<bin)
            previous = bin
        }
        self.ranges = ranges
    }

    deinit { vDSP_destroy_fftsetup(setup) }

    /// Adds samples; returns the bands once a full window has been heard.
    func push(_ samples: [Float]) -> [Float]? {
        guard !samples.isEmpty else { return nil }
        if samples.count >= Self.size {
            ring = Array(samples.suffix(Self.size))
        } else {
            ring.removeFirst(samples.count)
            ring.append(contentsOf: samples)
        }
        filled = min(Self.size, filled + samples.count)
        return filled == Self.size ? bands() : nil
    }

    func bands() -> [Float] {
        let half = Self.size / 2
        var windowed = [Float](repeating: 0, count: Self.size)
        vDSP.multiply(ring, window, result: &windowed)
        var real = [Float](repeating: 0, count: half)
        var imaginary = [Float](repeating: 0, count: half)
        var power = [Float](repeating: 0, count: half)
        real.withUnsafeMutableBufferPointer { realPointer in
            imaginary.withUnsafeMutableBufferPointer { imaginaryPointer in
                var split = DSPSplitComplex(realp: realPointer.baseAddress!, imagp: imaginaryPointer.baseAddress!)
                windowed.withUnsafeBufferPointer { input in
                    input.baseAddress!.withMemoryRebound(to: DSPComplex.self, capacity: half) { complex in
                        vDSP_ctoz(complex, 2, &split, 1, vDSP_Length(half))
                    }
                }
                vDSP_fft_zrip(setup, &split, 1, vDSP_Length(10), FFTDirection(FFT_FORWARD))
                vDSP_zvmags(&split, 1, &power, 1, vDSP_Length(half))
            }
        }
        // vDSP's forward real FFT is scaled by 2; with the window's sum, a full-scale sine
        // lands near 0 dB.
        let scale = Float(Self.size) * 0.5
        return ranges.map { range in
            let total = range.reduce(Float(0)) { $0 + (power.indices.contains($1) ? power[$1] : 0) }
            let level = 10 * log10(total / (scale * scale) + 1e-12)
            return min(1, max(0, (level - Self.floor) / (Self.ceiling - Self.floor)))
        }
    }

    /// Loudness of a stretch, 0…1 RMS.
    static func rms(_ samples: ArraySlice<Float>) -> Float {
        guard !samples.isEmpty else { return 0 }
        return samples.withUnsafeBufferPointer { vDSP.rootMeanSquare($0) }
    }
}
