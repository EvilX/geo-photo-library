// Декодер HEIC для ядра: системный ImageIO вместо libheif.
import CGeoPhotoCore
import Foundation
import ImageIO

/// Уже повёрнутое изображение с короткой стороной не меньше `minSide` (для квадратной
/// миниатюры). Вызывается из потоков сканера. Буфер выделяется `malloc` — его освободит ядро.
let decodeHeif: GplHeifDecoder = { path, minSide, out in
    guard let path, let out else { return false }
    let url = URL(fileURLWithFileSystemRepresentation: path, isDirectory: false, relativeTo: nil)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
          let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let pixelW = props[kCGImagePropertyPixelWidth] as? Int,
          let pixelH = props[kCGImagePropertyPixelHeight] as? Int,
          pixelW > 0, pixelH > 0
    else { return false }

    // ImageIO ограничивает длинную сторону — подбираем её так, чтобы короткая была не меньше minSide.
    let (short, long) = (min(pixelW, pixelH), max(pixelW, pixelH))
    let maxPixel = short <= Int(minSide) ? long : Int((Double(minSide) * Double(long) / Double(short)).rounded(.up))
    let options: [CFString: Any] = [
        kCGImageSourceCreateThumbnailFromImageAlways: true,
        // Поворот из контейнера (irot) и EXIF применяет ImageIO.
        kCGImageSourceCreateThumbnailWithTransform: true,
        kCGImageSourceThumbnailMaxPixelSize: maxPixel,
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return false }

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

    // Размеры в свойствах — до поворота: если миниатюра легла иначе, меняем их местами.
    let rotated = (w > h) != (pixelW > pixelH)
    out.pointee = GplImage(
        rgba: buffer.assumingMemoryBound(to: UInt8.self),
        width: UInt32(w),
        height: UInt32(h),
        orig_width: UInt32(rotated ? pixelH : pixelW),
        orig_height: UInt32(rotated ? pixelW : pixelH)
    )
    return true
}
