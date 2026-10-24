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
        // batao9 のフォークの c228776（旧辞書 azooKey_dictionary_storage@b05798b の読み込みに対応済み）に、
        // 確定のたびに Zenzai の context を作り直さない変更（本家 #350 と同じ・#13）と、
        // 本家の修正 3 件（#365 数詞・#363 明治の年・#367 壊れた学習データで落ちない、#28）を足した版
        .package(
            url: "https://github.com/00kuon00/AzooKeyKanaKanjiConverter",
            revision: "ae233d79e5ae42a9d190d4c71b410a8f724c6bce",
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
