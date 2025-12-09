// IndexBuilderDialog.qml - Dialog for building/rebuilding index

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.glint.app

Dialog {
    id: indexBuilderDialog
    title: qsTr("Build Index")
    modal: true
    standardButtons: Dialog.Ok | Dialog.Cancel
    
    width: 450
    height: 400
    
    anchors.centerIn: parent
    
    // Volume list model
    property var volumes: []
    property var selectedVolumes: []
    
    Component.onCompleted: {
        refreshVolumes()
    }
    
    function refreshVolumes() {
        var volumeString = GlintController.get_available_volumes()
        var configuredVolumes = GlintController.get_configured_volumes()
        var configuredList = configuredVolumes ? configuredVolumes.split(",") : []
        
        volumes = []
        selectedVolumes = []
        
        if (!volumeString || volumeString.length === 0) {
            volumeRepeater.model = volumes
            return
        }
        
        var volumeArray = volumeString.split(";")
        for (var i = 0; i < volumeArray.length; i++) {
            var parts = volumeArray[i].split("|")
            if (parts.length >= 3) {
                var letter = parts[0]
                // Pre-select volumes that are already configured, or all if none configured
                var isSelected = configuredList.length === 0 || configuredList.indexOf(letter) !== -1
                volumes.push({
                    letter: letter,
                    label: parts[1],
                    size: parts[2],
                    selected: isSelected
                })
                if (isSelected) {
                    selectedVolumes.push(letter)
                }
            }
        }
        volumeRepeater.model = volumes
    }
    
    ColumnLayout {
        anchors.fill: parent
        spacing: 16
        
        Label {
            text: qsTr("Select volumes to index:")
            font.pointSize: 11
            font.bold: true
        }
        
        // Volume list
        ScrollView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            
            ColumnLayout {
                width: parent.width
                spacing: 8
                
                Repeater {
                    id: volumeRepeater
                    
                    delegate: CheckBox {
                        text: "%1: (%2) - %3".arg(modelData.letter).arg(modelData.label).arg(modelData.size)
                        checked: modelData.selected
                        onCheckedChanged: {
                            if (checked && selectedVolumes.indexOf(modelData.letter) === -1) {
                                selectedVolumes.push(modelData.letter)
                            } else if (!checked) {
                                var idx = selectedVolumes.indexOf(modelData.letter)
                                if (idx !== -1) {
                                    selectedVolumes.splice(idx, 1)
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // Service option
        GroupBox {
            title: qsTr("Background Service")
            Layout.fillWidth: true
            
            ColumnLayout {
                anchors.fill: parent
                
                CheckBox {
                    id: enableServiceCheckbox
                    text: qsTr("Enable real-time index updates")
                    checked: true
                }
                
                Label {
                    text: qsTr("The service monitors file changes and keeps the index up-to-date.")
                    wrapMode: Text.WordWrap
                    opacity: 0.7
                    font.pointSize: 9
                    Layout.fillWidth: true
                }
            }
        }
        
        // Progress indicator
        ProgressBar {
            id: progressBar
            Layout.fillWidth: true
            visible: GlintController.is_indexing
            indeterminate: true
        }
        
        Label {
            text: GlintController.is_indexing ? qsTr("Indexing in progress...") : ""
            visible: GlintController.is_indexing
        }
    }
    
    onOpened: {
        refreshVolumes()
    }
    
    onAccepted: {
        if (selectedVolumes.length === 0) {
            // Show warning
            return
        }
        
        // Build list of selected volume letters
        var volumeList = []
        for (var i = 0; i < selectedVolumes.length; i++) {
            volumeList.push(selectedVolumes[i])
        }
        
        // Start async indexing and open progress dialog
        GlintController.start_indexing(volumeList.join(","))
        indexingProgressDialog.open()
        
        // Start service if requested (after indexing completes)
        if (enableServiceCheckbox.checked) {
            indexingProgressDialog.startServiceWhenDone = true
        }
    }
}
