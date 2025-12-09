// SearchBar.qml - Search input component

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.glint.app

RowLayout {
    id: root
    spacing: 8
    
    property alias text: searchField.text
    
    function focusSearchField() {
        searchField.forceActiveFocus()
    }
    
    // Search icon
    Label {
        text: "🔍"
        font.pointSize: 14
    }
    
    // Search input field
    TextField {
        id: searchField
        Layout.fillWidth: true
        placeholderText: qsTr("Search files and folders...")
        font.pointSize: 12
        selectByMouse: true
        
        // Real-time search as user types
        onTextChanged: {
            GlintController.query = text
            searchTimer.restart()
        }
        
        // Debounce search to avoid too many updates
        Timer {
            id: searchTimer
            interval: 150
            onTriggered: GlintController.search()
        }
        
        // Search on Enter key
        Keys.onReturnPressed: {
            searchTimer.stop()
            GlintController.search()
        }
        
        Keys.onEscapePressed: {
            if (text !== "") {
                text = ""
                GlintController.clear_search()
            }
        }
        
        // Navigate to results with down arrow
        Keys.onDownPressed: {
            resultsView.forceActiveFocus()
        }
    }
    
    // Clear button
    Button {
        text: "✕"
        flat: true
        visible: searchField.text !== ""
        onClicked: {
            searchField.text = ""
            GlintController.clear_search()
            searchField.forceActiveFocus()
        }
    }
    
    // Result count
    Label {
        text: GlintController.result_count > 0 ? 
              qsTr("%1 results").arg(GlintController.result_count) : ""
        font.pointSize: 10
        opacity: 0.7
    }
}
