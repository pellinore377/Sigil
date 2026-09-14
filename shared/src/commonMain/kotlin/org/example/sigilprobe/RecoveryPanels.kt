package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties

@Composable
fun RecoverySetup(secret:String,busy:Boolean,dismiss:()->Unit,enable:()->Unit,properties:DialogProperties=DialogProperties(),copy:(()->Unit)?=null) {
    var saved by remember(secret){mutableStateOf(false)}
    var check by remember(secret){mutableStateOf("")}
    AlertDialog(onDismissRequest={if(!busy)dismiss()},properties=properties,title={Text("Save your recovery key")},text={Column(Modifier.verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Text("Keep this in your password manager or write it down somewhere safe. Your server cannot replace it. Anyone with this key and access to your backup can read that history.")
        if(copy==null)androidx.compose.foundation.text.selection.SelectionContainer {Text(secret.chunked(4).joinToString(" "),fontFamily=LocalCodeFont.current)}
        else Text(secret.chunked(4).joinToString(" "),fontFamily=LocalCodeFont.current)
        copy?.let {SigilTextButton(it){Text("Copy for one minute")}}
        Row {Checkbox(saved,{saved=it},enabled=!busy);Text("I saved my recovery key.",Modifier.padding(top=12.dp))}
        OutlinedTextField(check,{if(it.length<=8)check=it},label={Text("Last 8 characters of your saved key")},singleLine=true,enabled=!busy)
    }},confirmButton={SigilTextButton(enable,enabled=saved && check==secret.takeLast(8) && !busy){Text("Enable encrypted backups")}},dismissButton={SigilTextButton(dismiss,enabled=!busy){Text("Cancel")}})
}

@Composable
fun RecoveryRestore(busy:Boolean,issue:String?,dismiss:()->Unit,restore:(String)->Unit,properties:DialogProperties=DialogProperties()) {
    var secret by remember{mutableStateOf("")};var reviewed by remember{mutableStateOf(false)}
    AlertDialog(onDismissRequest={if(!busy)dismiss()},properties=properties,title={Text("Restore encrypted history")},text={Column(Modifier.verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Text("Enter the recovery key you saved for this account. It stays on this device and is never sent to the server.")
        OutlinedTextField(secret,{if(it.length<=256)secret=it.filterNot(Char::isWhitespace).lowercase()},label={Text("Recovery key")},singleLine=true,enabled=!busy,visualTransformation=PasswordVisualTransformation(),keyboardOptions=KeyboardOptions(autoCorrectEnabled=false,keyboardType=KeyboardType.Password))
        Text("This device can verify the backup’s integrity, but cannot independently confirm that the server supplied the newest backup. Restore only from a server you trust.")
        Row {Checkbox(reviewed,{reviewed=it},enabled=!busy);Text("I understand and want to restore this backup.",Modifier.padding(top=12.dp))}
        Text("Restoring history does not approve contacts or copy a previous device’s messaging keys.",style=MaterialTheme.typography.bodySmall)
        issue?.let {Text(it,color=MaterialTheme.colorScheme.error)}
        if(busy)LinearProgressIndicator(Modifier.fillMaxWidth())
    }},confirmButton={SigilTextButton({restore(secret)},enabled=!busy && reviewed && validRecoveryKey(secret)){Text("Restore history")}},dismissButton={SigilTextButton(dismiss,enabled=!busy){Text("Cancel")}})
}

@Composable
fun AccountRecovery(sso:Boolean,busy:Boolean,issue:String?,dismiss:()->Unit,recover:(String,String?)->Unit,properties:DialogProperties=DialogProperties()) {
    var method by remember{mutableStateOf(if(sso)"sso" else "invitation")};var invitation by remember{mutableStateOf("")};var confirmed by remember{mutableStateOf(false)}
    AlertDialog(onDismissRequest={if(!busy)dismiss()},properties=properties,title={Text("Recover a lost account")},text={Column(Modifier.verticalScroll(rememberScrollState()),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Text("Recovery signs out all previous devices and creates a new encryption identity on this device. Your contacts will need to accept the replacement. Your recovery key restores backed-up history after sign-in.")
        Text("If you still have a signed-in device, you can use device linking instead.",style=MaterialTheme.typography.bodySmall)
        if(sso)Row {RadioButton(method=="sso",{method="sso"},enabled=!busy);Text("Sign in with SSO",Modifier.padding(top=12.dp))}
        Row {RadioButton(method=="invitation",{method="invitation"},enabled=!busy);Text("Administrator recovery invitation",Modifier.padding(top=12.dp))}
        if(method=="invitation")OutlinedTextField(invitation,{if(it.length<=256)invitation=it.trim()},label={Text("Recovery invitation")},singleLine=true,enabled=!busy,visualTransformation=PasswordVisualTransformation(),keyboardOptions=KeyboardOptions(autoCorrectEnabled=false,keyboardType=KeyboardType.Password))
        Row {Checkbox(confirmed,{confirmed=it},enabled=!busy);Text("Sign out my previous devices and recover this account.",Modifier.padding(top=12.dp))}
        issue?.let {Text(it,color=MaterialTheme.colorScheme.error)}
        if(busy)LinearProgressIndicator(Modifier.fillMaxWidth())
    }},confirmButton={SigilTextButton({recover(method,invitation.takeIf{method=="invitation"})},enabled=confirmed && !busy && (method=="sso" || validRecoveryKey(invitation))){Text("Continue recovery")}},dismissButton={SigilTextButton(dismiss,enabled=!busy){Text("Cancel")}})
}
private fun validRecoveryKey(value:String)=value.length==64 && value.all{it in '0'..'9' || it in 'a'..'f'}
