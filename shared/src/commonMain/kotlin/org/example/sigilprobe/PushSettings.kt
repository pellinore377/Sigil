package org.sigil

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.launch

internal data class PushConfiguration(val revision:String, val unified:Boolean, val contact:String,
    val project:String, val email:String, val vapid:String)
internal data class PushUpdate(val revision:String, val unified:Boolean, val contact:String,
    val disableGoogle:Boolean, val credentials:String?, val rotate:Boolean)

@Composable
@OptIn(ExperimentalLayoutApi::class)
internal fun PushSettings(read:suspend ()->PushConfiguration, save:suspend (PushUpdate)->PushConfiguration,
    field:@Composable (String,String,(String)->Unit,Boolean,Boolean)->Unit) {
    var configuration by remember { mutableStateOf<PushConfiguration?>(null) }
    var google by remember { mutableStateOf(false) }
    var unified by remember { mutableStateOf(false) }
    var contact by remember { mutableStateOf("") }
    var credentials by remember { mutableStateOf("") }
    var replacing by remember { mutableStateOf(false) }
    var advanced by remember { mutableStateOf(false) }
    var rotate by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf("") }
    var saved by remember { mutableStateOf(false) }
    var confirm by remember { mutableStateOf(false) }
    val scope=rememberCoroutineScope()
    fun load(value:PushConfiguration) {
        configuration=value;google=value.project.isNotEmpty();unified=value.unified;contact=value.contact
        credentials="";replacing=false;rotate=false;confirm=false
    }
    suspend fun operation(action:suspend ()->Unit) {
        if(busy)return
        busy=true;error="";saved=false
        try { action() }
        catch(cancelled:CancellationException) { throw cancelled }
        catch(failure:Exception) { error=failure.message ?: "Notification settings could not be saved. Reload and try again." }
        finally { busy=false }
    }
    fun refresh() { scope.launch(start=CoroutineStart.UNDISPATCHED) { operation { load(read()) } } }
    LaunchedEffect(Unit) { operation { load(read()) } }
    val current=configuration
    val editing=!busy && current!=null
    val needsCredentials=google && (current?.project.isNullOrEmpty() || replacing)
    val changed=current!=null && (google!=current.project.isNotEmpty() || unified!=current.unified ||
        contact.trim()!=current.contact || rotate || needsCredentials)
    val ready=editing && changed && (!unified || contact.isNotBlank()) && (!needsCredentials || credentials.isNotBlank())
    fun submit() {
        if(!ready)return
        val update=PushUpdate(checkNotNull(current).revision,unified,contact.trim(),!google,
            credentials.takeIf { needsCredentials },rotate)
        scope.launch(start=CoroutineStart.UNDISPATCHED) { operation {
            load(save(update));saved=true
        } }
    }
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(16.dp)) {
        Text("Notifications",style=MaterialTheme.typography.headlineMedium)
        Text("Enable delivery services for this server. Each device can choose a service or turn push off in its notification settings.")
        if(busy) LinearProgressIndicator(Modifier.fillMaxWidth())
        if(error.isNotEmpty()) Text(error,color=MaterialTheme.colorScheme.error)
        if(current==null) {
            if(!busy) SigilButton(::refresh) { Text("Retry notification settings") }
        } else {
            Row(verticalAlignment=Alignment.CenterVertically) {
                Switch(google,{google=it;credentials="";replacing=false;saved=false},enabled=editing,
                    modifier=Modifier.semantics { contentDescription="Enable Google notifications" })
                Text("Google notifications",Modifier.padding(start=12.dp))
            }
            AnimatedVisibility(google) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Text("Save the server credentials here, then add the public Android app configuration below. Both must use the same Firebase project. Saving configuration does not verify delivery.",style=MaterialTheme.typography.bodySmall)
                    if(current.project.isNotEmpty()) {
                        Text("Project: ${current.project}")
                        Text(current.email,style=MaterialTheme.typography.bodySmall)
                        SigilTextButton(onClick={replacing=!replacing;credentials="";saved=false},enabled=editing) {
                            Text(if(replacing) "Keep existing credentials" else "Replace credentials")
                        }
                    }
                    if(needsCredentials) {
                        field("Firebase service-account JSON",credentials,{if(it.length<=32768) { credentials=it;saved=false }},true,editing)
                        Text("Paste the service-account key downloaded from Firebase. It is sent only to this server and cleared from this form after saving. The Android google-services.json file is different.",style=MaterialTheme.typography.bodySmall)
                    }
                }
            }
            Row(verticalAlignment=Alignment.CenterVertically) {
                Switch(unified,{unified=it;saved=false},enabled=editing,
                    modifier=Modifier.semantics { contentDescription="Enable UnifiedPush" })
                Text("UnifiedPush",Modifier.padding(start=12.dp))
            }
            AnimatedVisibility(unified) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Text("For devices with a compatible push distributor installed. A contact identifies this server to the delivery service.",style=MaterialTheme.typography.bodySmall)
                    field("Contact · mailto:admin@example.org or HTTPS URL",contact,{contact=it;saved=false},false,editing)
                }
            }
            SigilTextButton(onClick={advanced=!advanced},enabled=!busy) { Text(if(advanced) "Hide advanced settings" else "Advanced notification settings") }
            AnimatedVisibility(advanced) {
                Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
                    Text("Existing private-network delivery exceptions are preserved. Manage them through the configuration API.",style=MaterialTheme.typography.bodySmall)
                    if(current.vapid.isNotEmpty()) {
                        Row(verticalAlignment=Alignment.CenterVertically) {
                            Checkbox(rotate,{rotate=it;saved=false},enabled=editing,
                                modifier=Modifier.semantics { contentDescription="Rotate UnifiedPush signing key" })
                            Text("Rotate UnifiedPush signing key",Modifier.padding(start=12.dp))
                        }
                        Text("Rotation requires devices to register again. Use this when replacing a compromised key.",style=MaterialTheme.typography.bodySmall)
                    }
                }
            }
            FlowRow(horizontalArrangement=Arrangement.spacedBy(12.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
                SigilButton(onClick={
                    if(rotate || (!google && current.project.isNotEmpty()) || (!unified && current.unified))confirm=true
                    else submit()
                },enabled=ready) { Text("Save notification settings") }
                SigilTextButton(onClick=::refresh,enabled=!busy) { Text("Reload") }
            }
            if(saved) Text("Saved. Device registration and delivery still need to complete.",style=MaterialTheme.typography.bodySmall)
        }
    }
    AnimatedVisibility(confirm) {
        Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {
            Text("Change notification delivery?",style=MaterialTheme.typography.headlineSmall)
            Text("Disabling a service stops its push delivery. Rotating the UnifiedPush key requires devices to register again. Existing messages stay available through sync.")
            Row(horizontalArrangement=Arrangement.spacedBy(12.dp)) {
                SigilTextButton(onClick={confirm=false},enabled=!busy) { Text("Cancel") }
                SigilButton(onClick={confirm=false;submit()},enabled=ready) { Text("Save changes") }
            }
        }
    }
}
