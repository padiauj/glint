//! Build script for cxx-qt integration.
//!
//! This generates the C++ bridge code for Qt/QML integration.

use cxx_qt_build::{CxxQtBuilder, QmlModule};

fn main() {
    CxxQtBuilder::new()
        .qt_module("Core")
        .qt_module("Gui")
        .qt_module("Qml")
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qml_module(QmlModule {
            uri: "org.glint.app",
            rust_files: &["src/bridge.rs"],
            qml_files: &[
                "qml/Main.qml",
                "qml/SearchBar.qml",
                "qml/ResultsView.qml",
                "qml/SettingsDialog.qml",
                "qml/IndexBuilderDialog.qml",
                "qml/AboutDialog.qml",
            ],
            ..Default::default()
        })
        .build();
}
