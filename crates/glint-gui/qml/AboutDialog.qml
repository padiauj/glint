// AboutDialog.qml - About dialog

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: aboutDialog
    title: qsTr("About Glint")
    modal: true
    standardButtons: Dialog.Ok
    
    width: 350
    height: 280
    
    anchors.centerIn: parent
    
    ColumnLayout {
        anchors.fill: parent
        spacing: 16
        
        // Logo/Icon
        Label {
            text: "⚡"
            font.pointSize: 48
            Layout.alignment: Qt.AlignHCenter
        }
        
        // Title
        Label {
            text: "Glint"
            font.pointSize: 24
            font.bold: true
            Layout.alignment: Qt.AlignHCenter
        }
        
        Label {
            text: qsTr("Fast File Search")
            font.pointSize: 12
            opacity: 0.7
            Layout.alignment: Qt.AlignHCenter
        }
        
        Label {
            text: "Version 0.1.0"
            font.pointSize: 10
            opacity: 0.7
            Layout.alignment: Qt.AlignHCenter
        }
        
        Item { height: 8 }
        
        Label {
            text: qsTr("A blazingly fast file search tool\ninspired by Voidtools Everything.")
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignHCenter
        }
        
        Item { height: 8 }
        
        Label {
            text: "<a href='https://github.com/padiauj/glint'>GitHub Repository</a>"
            textFormat: Text.RichText
            onLinkActivated: Qt.openUrlExternally(link)
            Layout.alignment: Qt.AlignHCenter
            
            MouseArea {
                anchors.fill: parent
                acceptedButtons: Qt.NoButton
                cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor
            }
        }
        
        Label {
            text: qsTr("Licensed under MIT or Apache-2.0")
            font.pointSize: 9
            opacity: 0.5
            Layout.alignment: Qt.AlignHCenter
        }
        
        Label {
            text: qsTr("Qt used under LGPL license")
            font.pointSize: 9
            opacity: 0.5
            Layout.alignment: Qt.AlignHCenter
        }
        
        Item { Layout.fillHeight: true }
    }
}
