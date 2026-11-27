// Декодер кадров видео для ядра: AVFoundation вместо ffmpeg.
import AVFoundation
import CGeoPhotoCore
import Foundation

/// Кадр на 1-й секунде видео (у коротких — первый), уже повёрнутый, с короткой стороной не
/// меньше `minSide`. Вызывается из потоков сканера и ждёт результат; буфер выделяется `malloc` —
/// его освободит ядро. Размеры оригинала ядро берёт из контейнера сам.
let decodeVideoFrame: GplHeifDecoder = { path, minSide, out in
    guard let path, let out else { return false }
    let url = URL(fileURLWithFileSystemRepresentation: path, isDirectory: false, relativeTo: nil)
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
    generator.appliesPreferredTrackTransform = true
    // Кадр вписывается в квадрат — короткая сторона не меньше minSide при пропорциях до 1:4.
    let side = CGFloat(minSide) * 4
    generator.maximumSize = CGSize(width: side, height: side)

    var frame: CGImage?
    for seconds in [1.0, 0.0] {
        let done = DispatchSemaphore(value: 0)
        generator.generateCGImageAsynchronously(for: CMTime(seconds: seconds, preferredTimescale: 600)) { image, _, _ in
            frame = image
            done.signal()
        }
        done.wait()
        if frame != nil { break }
    }
    guard let image = frame else { return false }

    let (w, h) = (image.width, image.height)
    guard w > 0, h > 0, let buffer = malloc(w * h * 4) else { return false }
    guard let ctx = CGContext(
        data: buffer,
        width: w,
        height: h,
        bitsPerComponent: 8,
        bytesPerRow: w * 4,
        space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
        bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else {
        free(buffer)
        return false
    }
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    out.pointee = GplImage(
        rgba: buffer.assumingMemoryBound(to: UInt8.self),
        width: UInt32(w),
        height: UInt32(h),
        orig_width: 0,
        orig_height: 0
    )
    return true
}
