// IndexingProgressDialog.qml - Progress dialog for indexing operation

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import org.glint.app

Dialog {
    id: indexingProgressDialog
    title: qsTr("Indexing Progress")
    modal: true
    closePolicy: Popup.NoAutoClose
    
    width: 450
    height: 200
    
    anchors.centerIn: parent
    
    property bool startServiceWhenDone: false
    
    // Timer to poll for progress updates
    Timer {
        id: progressTimer
        interval: 100
        running: indexingProgressDialog.visible && GlintController.is_indexing
        repeat: true
        onTriggered: {
            GlintController.check_indexing_progress()
        }
    }
    
    // Watch for indexing completion
    Connections {
        target: GlintController
        function onIs_indexingChanged() {
            if (!GlintController.is_indexing && indexingProgressDialog.visible) {
                // Indexing complete
                progressTimer.stop()
                
                // Start service if requested
                if (startServiceWhenDone && !GlintController.service_running) {
                    GlintController.toggle_service()
                }
                startServiceWhenDone = false
                
                // Close after a short delay to show completion
                closeTimer.start()
            }
        }
    }
    
    Timer {
        id: closeTimer
        interval: 1500
        onTriggered: indexingProgressDialog.close()
    }
    
    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 16
        
        // Status text
        Label {
            text: GlintController.index_progress_text
            font.pointSize: 12
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
        }
        
        // Progress bar
        ProgressBar {
            id: progressBar
            Layout.fillWidth: true
            from: 0
            to: 100
            value: GlintController.index_progress
        }
        
        // Volume progress text
        Label {
            text: GlintController.index_total_volumes > 0 ?
                  qsTr("Volume %1 of %2").arg(GlintController.index_current_volume).arg(GlintController.index_total_volumes) :
                  ""
            font.pointSize: 10
            opacity: 0.7
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
        }
        
        // Completion message
        Label {
            visible: !GlintController.is_indexing && GlintController.index_progress === 100
            text: qsTr("Indexing complete! Found %1 files.").arg(GlintController.index_count.toLocaleString())
            font.pointSize: 11
            font.bold: true
            color: "#4caf50"
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
        }
        
        Item { Layout.fillHeight: true }
    }
    
    // No standard buttons - dialog closes automatically when done
    standardButtons: Dialog.NoButton
    
    onOpened: {
        startServiceWhenDone = false
        progressTimer.start()
    }
    
    onClosed: {
        progressTimer.stop()
        closeTimer.stop()
    }
}
