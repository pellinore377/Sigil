package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.semantics.*
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.background

@Composable internal fun NavigationPane(state:MessengerState,collection:String,collectionChanged:(String)->Unit,
    selected:Set<String>,select:(String)->Unit,clear:()->Unit,collections:()->Unit,
    open:(String)->Unit,navigate:(String)->Unit,command:Command,read:(String)->String?, page:String = "inbox") {
    val section = when {
        page == "calls" -> "calls"
        page == "notes" -> "notes"
        page == "settings" || page.startsWith("appearance") || page in listOf("device", "profile", "privacy", "notifications", "storage", "about") -> "settings"
        else -> "inbox"
    }
    var collapsed by remember { mutableStateOf(false) }
    val paneWidth by animateDpAsState(if (collapsed) 72.dp else 336.dp, LocalMotion.current.tween(MotionMillis), label = "Directory width")
    Surface(Modifier.width(paneWidth).fillMaxHeight(), shape = RoundedCornerShape(28.dp), color = MaterialTheme.colorScheme.background) {
      Column {
        if(!collapsed && selected.isNotEmpty())MainHeader("inbox",false,"",{},selected,state,command,clear,collections,{navigate("search")},{navigate("new")},{navigate("inbox")})
        else Row(Modifier.fillMaxWidth().heightIn(min = mainHeaderHeight()).padding(horizontal = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Symbol(if (collapsed) "chevron_right" else "chevron_left", if (collapsed) "Expand directory" else "Collapse directory") { collapsed = !collapsed }
            if (!collapsed) {
                MainHeaderTitle("Sigil", Modifier.weight(1f).padding(start = 8.dp))
                Symbol("search", "Search") { navigate("search") }
                SigilIconButton({ navigate("new") }) { Glyph("edit_square", 24, "New conversation", filled = false) }
            }
        }
        if (collapsed) {
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { SigilIconButton({ navigate("new") }) { Glyph("edit_square", 24, "New conversation", filled = false) } }
            Box(Modifier.weight(1f)) {}
        } else Box(Modifier.weight(1f).fillMaxWidth()) {
            Inbox(state,collection,collectionChanged,selected,select,open,read)
        }
        if (collapsed) Column(Modifier.fillMaxWidth().padding(bottom = 12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            NavigationIcon("chat_bubble","Messages",section == "inbox"){navigate("inbox")}
            if (LocalClientFeatures.current.calls) NavigationIcon("call","Calls",section == "calls"){navigate("calls")}
            NavigationIcon("description","Notes",section == "notes"){navigate("notes")}
            NavigationIcon("settings","Settings",section == "settings"){navigate("settings")}
        } else Row(Modifier.fillMaxWidth().padding(vertical=12.dp),horizontalArrangement=Arrangement.SpaceEvenly) {
            NavigationIcon("chat_bubble","Messages",section == "inbox"){navigate("inbox")}
            if (LocalClientFeatures.current.calls) NavigationIcon("call","Calls",section == "calls"){navigate("calls")}
            NavigationIcon("description","Notes",section == "notes"){navigate("notes")}
            NavigationIcon("settings","Settings",section == "settings"){navigate("settings")}
        }
      }
    }
}

@Composable internal fun ConversationWelcome() {
    Box(Modifier.fillMaxSize().padding(40.dp), contentAlignment = Alignment.Center) {
        Text("Sigil", style = MaterialTheme.typography.displayLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable internal fun NavigationIcon(icon: String, label: String, active: Boolean, action: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Box(Modifier.size(width = 58.dp, height = 48.dp)
        .background(if (active) colors.primaryContainer else androidx.compose.ui.graphics.Color.Transparent, RoundedCornerShape(18.dp))
        .clip(RoundedCornerShape(18.dp)).selectable(active, role = Role.Tab, onClick = action).semantics { contentDescription = label }, contentAlignment = Alignment.Center) {
        CompositionLocalProvider(LocalContentColor provides if (active) colors.onPrimaryContainer else colors.onSurfaceVariant) {
            Glyph(icon, 25, filled = active)
        }
    }
}
