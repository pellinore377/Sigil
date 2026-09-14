package org.sigil

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlinx.serialization.json.*

@Composable fun LinkPanel(raw:String,busy:Boolean,issue:String?,command:(String,String?)->Unit,scanner:@Composable ((String)->Unit)->Unit) {
    val flow=remember(raw){Json.parseToJsonElement(raw).jsonObject}
    val stage=flow.string("stage")
    var scanning by remember(stage){mutableStateOf(stage=="scan_offer")}
    var matched by remember(stage){mutableStateOf(false)}
    val canCancel=flow.bool("can_cancel",true)
    val close={if(!busy)command(if(stage=="done")"close" else if(canCancel)"cancel" else "pause",null)}
    Surface(Modifier.fillMaxSize(),shape=MaterialTheme.shapes.extraLarge) {
        Column(Modifier.fillMaxSize().systemBarsPadding().padding(24.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(16.dp)) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                Text("Link a device",Modifier.weight(1f),style=MaterialTheme.typography.headlineMedium)
                SigilTextButton(close,enabled=!busy){Text(if(stage=="done")"Done" else if(canCancel)"Cancel" else "Finish later")}
            }
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState()),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(20.dp)) {
                Text(when(stage){
                    "show_offer"->"On your existing device, open Settings, then Devices, then Link a new device. Scan this code with that device."
                    "scan_offer"->"Scan the code shown by your new device. Keep both devices with you throughout setup."
                    "show_proposal"->"Now scan this code with your new device. Compare the symbols shown on both screens."
                    "confirm_join","confirm_sponsor"->"Check that these symbols match on both devices. Only approve a device you have with you."
                    "show_response"->"Scan this final code with your existing device and approve the link there. Then finish here."
                    "authorize"->"Your approval is saved. Retry to finish registering the device."
                    "cancelling"->"Cancellation is pending. Retry to make sure the server cancels this link."
                    "done"->"Your device is linked."
                    else->"Preparing a secure link…"
                })
                if(scanning && !busy)scanner {scanning=false;command("scan",it)}
                else if(flow.containsKey("cells"))QrGrid(flow.long("width").toInt(),flow.string("cells"),"Device linking QR code")
                val emoji=flow["emoji"]?.takeUnless{it==JsonNull}?.jsonArray?.map {it.jsonPrimitive.content}.orEmpty()
                if(emoji.isNotEmpty())Text(emoji.joinToString(" "),style=MaterialTheme.typography.headlineMedium)
                flow.optional("account")?.let {Text(it,style=MaterialTheme.typography.titleMedium)}
                issue?.let {Text(it,color=MaterialTheme.colorScheme.error)}
                if(busy)CircularProgressIndicator(Modifier.size(24.dp))
            }
            when(stage){
                "scan_offer","show_offer","show_proposal"->if(!scanning)SigilButton({scanning=true},enabled=!busy){Text("Scan the other device")}
                "confirm_join","confirm_sponsor"->{Row(verticalAlignment=Alignment.CenterVertically){Checkbox(matched,{matched=it},enabled=!busy);Text("The symbols match on both devices.",Modifier.weight(1f))};SigilButton({command("confirm",null)},enabled=matched&&!busy){Text("Approve this device")}}
                "show_response"->SigilButton({command("finish",null)},enabled=!busy){Text("Finish linking")}
                "prepare_offer","authorize","cancelling"->SigilButton({command("retry",null)},enabled=!busy){Text("Retry")}
                "done"->SigilButton(close,enabled=!busy){Text("Continue")}
            }
            if(scanning && stage!="scan_offer")SigilTextButton({scanning=false}){Text("Show my code")}
        }
    }
}
@Composable fun QrGrid(width:Int,cells:String,label:String) {
    require(width in 1..177 && cells.length==width*width)
    Canvas(Modifier.widthIn(max=440.dp).fillMaxWidth().aspectRatio(1f).semantics{contentDescription=label}) {
        drawRect(Color.White)
        val unit=kotlin.math.floor(size.minDimension/(width+8))
        val origin=Offset((size.width-unit*width)/2,(size.height-unit*width)/2)
        cells.forEachIndexed {i,c->if(c=='1')drawRect(Color.Black,origin+Offset(i%width*unit,i/width*unit),Size(unit,unit))}
    }
}

@Composable fun ContactPanel(raw:String,busy:Boolean,issue:String?,command:(String,String?)->Unit,scanner:@Composable ((String)->Unit)->Unit) {
    val flow=remember(raw){Json.parseToJsonElement(raw).jsonObject}
    val scanning=flow.string("stage")=="scan"
    val unavailable=flow.bool("consumed")||flow.bool("expired")
    Surface(Modifier.fillMaxSize(),shape=MaterialTheme.shapes.extraLarge) {
        Column(Modifier.fillMaxSize().systemBarsPadding().padding(24.dp),horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(16.dp)) {
            Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                Text(if(flow.containsKey("review"))"Confirm identity" else "Connect in person",Modifier.weight(1f),style=MaterialTheme.typography.headlineMedium)
                SigilTextButton({command("close",null)},enabled=!busy){Text("Close")}
            }
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(16.dp)) {
                Text(if(scanning)"Scan the contact code on the other person’s screen." else "Have the other person scan this code to connect. One person can use it, within ten minutes.")
                when {
                    busy->CircularProgressIndicator()
                    scanning->scanner {command("scan",it)}
                    flow.bool("consumed")->Text("Code scanned. You can close this screen.")
                    flow.bool("expired")->Text("This code expired. Show a new code to connect.")
                    flow.containsKey("cells")->QrGrid(flow.long("width").toInt(),flow.string("cells"),"Contact QR code")
                }
                issue?.let{Text(it,color=MaterialTheme.colorScheme.error)}
            }
            if(!flow.containsKey("review"))SigilButton({command(if(scanning||unavailable)"show" else "scan",null)},enabled=!busy){Text(if(scanning||unavailable)"Show my code" else "Scan a code")}
        }
    }
}
