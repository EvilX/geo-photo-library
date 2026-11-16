// swift-tools-version:5.10
// Нативный интерфейс macOS (SwiftUI + MapKit) поверх ядра на Rust.
// Сборка — через ./build-app.sh: он сначала собирает libgeo_photo_library.a.
import Foundation
import PackageDescription

/// Корень репозитория: статическая библиотека ядра лежит в его `target/release`.
let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().path

let package = Package(
    name: "GeoPhotoLibrary",
    platforms: [.macOS(.v14)],
    targets: [
        .target(name: "CGeoPhotoCore"),
        .executableTarget(
            name: "GeoPhotoLibrary",
            dependencies: ["CGeoPhotoCore"],
            linkerSettings: [
                .unsafeFlags(["-L\(repo)/target/release"]),
                .linkedLibrary("geo_photo_library"),
                // Системные библиотеки, которые нужны ядру (rustc --print native-static-libs).
                .linkedLibrary("iconv"),
                .linkedFramework("CoreFoundation"),
            ]
        ),
    ],
    swiftLanguageVersions: [.v5]
)
