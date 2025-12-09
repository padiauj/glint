// ResultsView.qml - Search results list view

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.glint.app

ListView {
    id: resultsView
    clip: true
    
    model: GlintController.result_count
    
    // Placeholder when no results
    Label {
        anchors.centerIn: parent
        visible: GlintController.result_count === 0 && GlintController.query !== ""
        text: qsTr("No results found")
        font.pointSize: 14
        opacity: 0.5
    }
    
    // Welcome message when no search
    Label {
        anchors.centerIn: parent
        visible: GlintController.result_count === 0 && GlintController.query === ""
        text: GlintController.index_count > 0 ?
              qsTr("Type to search %1 files").arg(GlintController.index_count.toLocaleString()) :
              qsTr("Build an index to start searching")
        font.pointSize: 14
        opacity: 0.5
    }
    
    delegate: ItemDelegate {
        id: resultDelegate
        width: resultsView.width
        height: 48
        
        // Get result data from controller
        property var resultData: {
            var data = GlintController.get_result(index)
            if (data && typeof data === "string") {
                var parts = data.split("|")
                return {
                    name: parts[0] || "",
                    path: parts[1] || "",
                    size: parts[2] || "",
                    modified: parts[3] || "",
                    isDir: parts[4] === "true"
                }
            }
            return { name: "", path: "", size: "", modified: "", isDir: false }
        }
        
        contentItem: RowLayout {
            spacing: 12
            
            // File/folder icon
            Label {
                text: resultData.isDir ? "📁" : "📄"
                font.pointSize: 16
            }
            
            // Name and path
            ColumnLayout {
                spacing: 2
                Layout.fillWidth: true
                
                Label {
                    text: resultData.name
                    font.pointSize: 11
                    font.bold: true
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                
                Label {
                    text: resultData.path
                    font.pointSize: 9
                    opacity: 0.7
                    elide: Text.ElideMiddle
                    Layout.fillWidth: true
                }
            }
            
            // Size
            Label {
                text: resultData.size
                font.pointSize: 10
                opacity: 0.7
                visible: !resultData.isDir && resultData.size !== ""
            }
            
            // Modified date
            Label {
                text: resultData.modified
                font.pointSize: 10
                opacity: 0.7
                visible: resultData.modified !== ""
            }
        }
        
        // Double-click to open
        onDoubleClicked: {
            GlintController.open_item(resultData.path)
        }
        
        // Context menu
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.RightButton
            onClicked: {
                resultsView.currentIndex = index
                contextMenu.popup()
            }
        }
    }
    
    // Context menu
    Menu {
        id: contextMenu
        
        Action {
            text: qsTr("Open")
            onTriggered: {
                var data = GlintController.get_result(resultsView.currentIndex)
                if (data) {
                    var parts = data.split("|")
                    GlintController.open_item(parts[1])
                }
            }
        }
        
        Action {
            text: qsTr("Open Containing Folder")
            onTriggered: {
                var data = GlintController.get_result(resultsView.currentIndex)
                if (data) {
                    var parts = data.split("|")
                    GlintController.open_folder(parts[1])
                }
            }
        }
        
        MenuSeparator {}
        
        Action {
            text: qsTr("Copy Full Path")
            onTriggered: {
                var data = GlintController.get_result(resultsView.currentIndex)
                if (data) {
                    var parts = data.split("|")
                    GlintController.copy_path(parts[1])
                }
            }
        }
    }
    
    // Scroll bar
    ScrollBar.vertical: ScrollBar {
        active: true
    }
    
    // Keyboard navigation
    Keys.onReturnPressed: {
        if (currentIndex >= 0) {
            var data = GlintController.get_result(currentIndex)
            if (data) {
                var parts = data.split("|")
                GlintController.open_item(parts[1])
            }
        }
    }
}
