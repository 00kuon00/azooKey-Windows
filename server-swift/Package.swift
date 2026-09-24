// swift-tools-version: 6.1
// The swift-tools-version declares the minimum version of Swift required to build this package.

import PackageDescription

let package = Package(
    name: "azookey-server",
    products: [
        // Products define the executables and libraries a package produces, making them visible to other packages.
        .library(
            name: "azookey-server",
            type: .dynamic,
            targets: ["azookey-server"]
        ),
        .library(name: "ffi", targets: ["azookey-server"])
    ],
    dependencies: [
        // Dependencies declare other packages that this package depends on.
        // .package(url: /* package url */, from: "1.0.0"),
        // c228776 は batao9 のフォークにだけある版（旧辞書 azooKey_dictionary_storage@b05798b の読み込みに対応済み）
        .package(
            url: "https://github.com/batao9/AzooKeyKanaKanjiConverter",
            revision: "c228776b0b869f81ee2a1031ff9dbd679f4b3cd9",
            traits: ["Zenzai"]
        )
    ],
    targets: [
        // Targets are the basic building blocks of a package, defining a module or a test suite.
        // Targets can depend on other targets in this package and products from dependencies.
        .target(name: "ffi"),
        .target(
            name: "azookey-server",
            dependencies: [
                .product(name: "KanaKanjiConverterModule", package: "azookeykanakanjiconverter"),
                // ユーザー辞書の読みをカタカナにする（toKatakana）
                .product(name: "SwiftUtils", package: "azookeykanakanjiconverter"),
                "ffi"
            ],
            // Zenzai トレイトの変換モジュールは C++ 相互運用でビルドされるため、利用側も合わせる
            swiftSettings: [.interoperabilityMode(.Cxx)]
        ),
        .testTarget(
            name: "azookey-serverTests",
            dependencies: ["azookey-server"],
            swiftSettings: [.interoperabilityMode(.Cxx)]
        ),
    ]
)
