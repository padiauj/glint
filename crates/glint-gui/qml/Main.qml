// Main.qml - Main application window for Glint
// Uses Qt Quick Controls 2 for modern UI components

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window
import org.glint.app

ApplicationWindow {
    id: window
    visible: true
    width: 900
    height: 600
    minimumWidth: 600
    minimumHeight: 400
    title: "Glint - Fast File Search"
    
    // Use system palette for native look
    palette.window: GlintController.dark_mode ? "#1e1e1e" : "#ffffff"
    palette.windowText: GlintController.dark_mode ? "#ffffff" : "#000000"
    palette.base: GlintController.dark_mode ? "#2d2d2d" : "#ffffff"
    palette.text: GlintController.dark_mode ? "#ffffff" : "#000000"
    palette.highlight: "#0078d4"
    palette.highlightedText: "#ffffff"
    
    // Menu bar
    menuBar: MenuBar {
        Menu {
            title: qsTr("&File")
            Action {
                text: qsTr("&Build Index...")
                onTriggered: indexBuilderDialog.open()
            }
            MenuSeparator {}
            Action {
                text: qsTr("&Settings...")
                shortcut: "Ctrl+,"
                onTriggered: settingsDialog.open()
            }
            MenuSeparator {}
            Action {
                text: qsTr("E&xit")
                shortcut: "Alt+F4"
                onTriggered: Qt.quit()
            }
        }
        
        Menu {
            title: qsTr("&Tools")
            Action {
                text: qsTr("&Reload Index")
                shortcut: "F5"
                onTriggered: GlintController.reload_index()
            }
            MenuSeparator {}
            Action {
                text: GlintController.service_running ? 
                      qsTr("Stop Background Service") : 
                      qsTr("Start Background Service")
                onTriggered: GlintController.toggle_service()
            }
        }
        
        Menu {
            title: qsTr("&Help")
            Action {
                text: qsTr("&About Glint")
                onTriggered: aboutDialog.open()
            }
        }
    }
    
    // Main content
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 8
        spacing: 8
        
        // Search bar
        SearchBar {
            id: searchBar
            Layout.fillWidth: true
        }
        
        // Results view
        ResultsView {
            id: resultsView
            Layout.fillWidth: true
            Layout.fillHeight: true
        }
        
        // Status bar
        RowLayout {
            Layout.fillWidth: true
            
            Label {
                text: GlintController.status_message
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            
            Label {
                text: GlintController.service_running ? "● Service Running" : "○ Service Stopped"
                color: GlintController.service_running ? "#4caf50" : "#9e9e9e"
                font.pointSize: 9
            }
        }
    }
    
    // Dialogs
    SettingsDialog {
        id: settingsDialog
    }
    
    IndexBuilderDialog {
        id: indexBuilderDialog
    }
    
    AboutDialog {
        id: aboutDialog
    }
    
    // Show index builder on first run if no volumes configured
    Component.onCompleted: {
        if (GlintController.needs_initial_setup()) {
            indexBuilderDialog.open()
        }
    }
    
    // Keyboard shortcuts
    Shortcut {
        sequence: "Ctrl+L"
        onActivated: searchBar.focusSearchField()
    }
    
    Shortcut {
        sequence: "Escape"
        onActivated: {
            if (searchBar.text !== "") {
                GlintController.clear_search()
                searchBar.text = ""
            }
        }
    }
}
