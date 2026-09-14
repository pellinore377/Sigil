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
import androidx.compose.ui.unit.sp
import kotlinx.serialization.json.*
import kotlinx.coroutines.delay

@Composable fun LinkPanel(raw:String,busy:Boolean,issue:String?,command:(String,String?)->Unit,scanner:@Composable ((String)->Unit)->Unit) {
    val flow=remember(raw){Json.parseToJsonElement(raw).jsonObject}
    val stage=flow.string("stage")
    var scanning by remember(stage){mutableStateOf(stage=="scan_offer")}
    val canCancel=flow.bool("can_cancel",true)
    val currentCommand by rememberUpdatedState(command)
    val currentBusy by rememberUpdatedState(busy)
    LaunchedEffect(stage) {
        if(stage in setOf("show_offer","exchanging","wait_approval"))while(true){
            delay(1500)
            if(!currentBusy)currentCommand("poll",null)
        }
    }
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
                    "exchanging"->"Connecting to your new device…"
                    "confirm_sponsor"->"Tap the emoji shown on your new device to approve it. Only link a device you have with you."
                    "wait_approval"->"Select this emoji on your phone. This device will sign in automatically."
                    "restart_required"->"Cancel this older linking attempt, then start again to use the single-scan flow."
                    "authorize"->"Your approval is saved. Retry to finish registering the device."
                    "cancelling"->"Cancellation is pending. Retry to make sure the server cancels this link."
                    "done"->"Your device is linked."
                    else->"Preparing a secure link…"
                })
                if(scanning && !busy)scanner {scanning=false;command("scan",it)}
                else if(flow.containsKey("cells"))QrGrid(flow.long("width").toInt(),flow.string("cells"),"Device linking QR code")
                val emoji=flow["emoji"]?.takeUnless{it==JsonNull}?.jsonArray?.map {it.jsonPrimitive.content}.orEmpty()
                if(emoji.isNotEmpty())Text(emoji.joinToString(" "),fontSize=64.sp)
                flow.optional("account")?.let {Text(it,style=MaterialTheme.typography.titleMedium)}
                issue?.let {Text(it,color=MaterialTheme.colorScheme.error)}
                if(busy && stage !in setOf("show_offer","exchanging","wait_approval"))CircularProgressIndicator(Modifier.size(24.dp))
            }
            when(stage){
                "scan_offer"->if(!scanning)SigilButton({scanning=true},enabled=!busy){Text("Scan the new device")}
                "confirm_sponsor"->{
                    val choices=flow["choices"]?.jsonArray?.map {it.jsonPrimitive.content}.orEmpty()
                    choices.chunked(3).forEach {row->
                        Row(horizontalArrangement=Arrangement.spacedBy(16.dp)) {
                            row.forEach {emoji->SigilButton({command("confirm",emoji)},Modifier.size(80.dp),enabled=!busy){Text(emoji,style=MaterialTheme.typography.headlineMedium)}}
                        }
                    }
                }
                "prepare_offer","authorize","cancelling"->SigilButton({command("retry",null)},enabled=!busy){Text("Retry")}
                "done"->SigilButton(close,enabled=!busy){Text("Continue")}
            }
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
