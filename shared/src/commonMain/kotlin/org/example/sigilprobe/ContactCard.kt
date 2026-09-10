package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.*

@Composable
internal fun ContactCard(contact: ContactContent,open: (() -> Unit)?) {
    var expanded by remember(contact) { mutableStateOf(false) }
    var identity by remember { mutableStateOf(false) }
    val clipboard=LocalClipboardManager.current
    Column(Modifier.widthIn(min=200.dp,max=280.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(8.dp)) { Glyph("person",24);Text("Contact",style=MaterialTheme.typography.labelLarge) }
        RichMessageText(contact.name)
        Text(contact.address,style=MaterialTheme.typography.bodyMedium)
        Row(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
            if(open!=null) SigilButton(open,modifier=Modifier.semantics { contentDescription="Message shared contact" }) { Text("Message") }
            SigilTextButton({expanded=true}) { Glyph("open_in_full",20);Spacer(Modifier.width(8.dp));Text("Open contact") }
        }
    }
    if(expanded) Dialog({expanded=false},DialogProperties(usePlatformDefaultWidth=false)) {
        Surface(Modifier.fillMaxSize()) {
            Column(Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(20.dp),verticalArrangement=Arrangement.spacedBy(20.dp)) {
                Row(verticalAlignment=Alignment.CenterVertically) { SigilIconButton({expanded=false}) { Glyph("close",24,"Close contact") };Text("Shared contact",style=MaterialTheme.typography.titleLarge) }
                Glyph("account_circle",64)
                RichMessageText(contact.name,style=MaterialTheme.typography.headlineSmall)
                SelectionContainer { Text(contact.address) }
                if(open!=null) SigilButton({expanded=false;open()},modifier=Modifier.semantics { contentDescription="Message shared contact" }) { Text("Message") }
                SigilTextButton({clipboard.setText(AnnotatedString(contact.address))}) { Glyph("content_copy",20);Spacer(Modifier.width(8.dp));Text("Copy address") }
                SigilTextButton({identity=!identity}) { Text(if(identity)"Hide details" else "More details") }
                Expandable(identity) {
                    Text("Shared identity reference",style=MaterialTheme.typography.labelLarge)
                    SelectionContainer { Text(contact.identity.chunked(4).joinToString(" "),fontFamily=LocalCodeFont.current,style=MaterialTheme.typography.bodySmall) }
                    if(contact.vcard!=null) SigilTextButton({clipboard.setText(AnnotatedString(contact.vcard))}) { Text("Copy vCard") }
                }
            }
        }
    }
}
