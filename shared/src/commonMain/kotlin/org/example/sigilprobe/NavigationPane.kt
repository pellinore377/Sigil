package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable internal fun NavigationPane(state:MessengerState,page:String,query:String,queryChanged:(String)->Unit,
    category:String,categoryChanged:(String)->Unit,collection:String,collectionChanged:(String)->Unit,
    selected:Set<String>,select:(String)->Unit,clear:()->Unit,collections:()->Unit,
    open:(String)->Unit,notes:(String)->Unit,navigate:(String)->Unit,command:Command,
    read:(String)->String?,write:(String,String)->Unit) {
    val section=if(page in listOf("search","notes"))page else "inbox"
    Column(Modifier.width(360.dp).fillMaxHeight().background(MaterialTheme.colorScheme.surface)) {
        MainHeader(section,false,query,queryChanged,selected,state,command,clear,collections,
            {navigate("search")},{navigate("notes")},{navigate("inbox")})
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when(section) {
                "search"->SearchPage(state,query,category,categoryChanged,open,command)
                "notes"->NotesGrid(state,query,read,write,notes,command)
                else->Inbox(state,collection,collectionChanged,selected,select,open,read)
            }
        }
        SigilButton({navigate("new")},Modifier.fillMaxWidth().padding(horizontal=20.dp,vertical=12.dp)) {
            Glyph("edit_square",20);Spacer(Modifier.width(10.dp));Text("New conversation")
        }
        Row(Modifier.fillMaxWidth().padding(bottom=12.dp),horizontalArrangement=Arrangement.SpaceEvenly) {
            Symbol("chat_bubble","Messages"){navigate("inbox")}
            Symbol("settings","Settings"){navigate("settings")}
        }
    }
}

@Composable internal fun ConversationWelcome() {
    Box(Modifier.fillMaxSize().padding(40.dp),contentAlignment=Alignment.Center) {
        Column(horizontalAlignment=Alignment.CenterHorizontally,verticalArrangement=Arrangement.spacedBy(16.dp)) {
            Text("Sigil",style=MaterialTheme.typography.displayLarge)
            Text("Choose a conversation, or start a new one.",style=MaterialTheme.typography.bodyLarge,color=MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
