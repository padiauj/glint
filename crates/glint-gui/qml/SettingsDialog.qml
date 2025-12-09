// SettingsDialog.qml - Application settings dialog

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.glint.app

Dialog {
    id: settingsDialog
    title: qsTr("Settings")
    modal: true
    standardButtons: Dialog.Ok | Dialog.Cancel
    
    width: 400
    height: 300
    
    anchors.centerIn: parent
    
    ColumnLayout {
        anchors.fill: parent
        spacing: 16
        
        // Appearance section
        GroupBox {
            title: qsTr("Appearance")
            Layout.fillWidth: true
            
            ColumnLayout {
                anchors.fill: parent
                
                CheckBox {
                    id: darkModeCheckbox
                    text: qsTr("Dark Mode")
                    checked: GlintController.dark_mode
                }
            }
        }
        
        // Search section
        GroupBox {
            title: qsTr("Search")
            Layout.fillWidth: true
            
            ColumnLayout {
                anchors.fill: parent
                
                RowLayout {
                    Label {
                        text: qsTr("Maximum results:")
                    }
                    SpinBox {
                        id: maxResultsSpinBox
                        from: 50
                        to: 1000
                        stepSize: 50
                        value: 100
                    }
                }
            }
        }
        
        // Index section
        GroupBox {
            title: qsTr("Index")
            Layout.fillWidth: true
            
            ColumnLayout {
                anchors.fill: parent
                
                Label {
                    text: qsTr("Indexed files: %1").arg(GlintController.index_count.toLocaleString())
                }
                
                Button {
                    text: qsTr("Rebuild Index...")
                    onClicked: {
                        settingsDialog.close()
                        indexBuilderDialog.open()
                    }
                }
            }
        }
        
        Item { Layout.fillHeight: true }
    }
    
    onAccepted: {
        GlintController.dark_mode = darkModeCheckbox.checked
        // Save other settings as needed
    }
}
