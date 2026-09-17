package org.sigil

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.grid.*
import androidx.compose.foundation.pager.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.*
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

internal val createItems=listOf(
    "Contact" to "person","Poll" to "ballot","Checklist" to "checklist","Recipe" to "restaurant",
    "Dice" to "casino","Coin" to "toll","Cards" to "playing_cards","Random Number" to "numbers",
    "Note" to "description","Task" to "assignment","Reminder" to "notifications_active","Timer" to "timer",
    "Table" to "table","Chart" to "bar_chart","Diagram" to "account_tree","Calculation" to "calculate","Conversion" to "swap_horiz",
    "Rating" to "star","Progress" to "data_usage","Math" to "functions","Recurring checklist" to "event_repeat",
    "Countdown" to "hourglass_bottom","Elapsed time" to "history","Color swatch" to "palette","Keyboard shortcut" to "keyboard",
    "Quote" to "format_quote","QR code" to "qr_code","ASCII art" to "draw","Translation" to "translate","Definition" to "dictionary",
    "Weather" to "partly_cloudy_day","Help" to "help"
)
private val createPages=createItems.filter {it.first!="Help"}.chunked(12)

@Composable
internal fun CreatePanel(back:()->Unit,open:(String)->Unit) {
    val pager=rememberPagerState {createPages.size}
    val scope=rememberCoroutineScope()
    val motionPolicy=LocalMotion.current
    val preferred=LocalComposerPanelHeight.current
    val density=LocalDensity.current
    BoxWithConstraints(Modifier.fillMaxWidth()) {
    // Header line box, the 48.dp dot row, the 8.dp top padding and the two 4.dp gaps.
    val chrome=with(density) {maxOf(48.dp,MaterialTheme.typography.titleMedium.lineHeight.toDp())}+64.dp
    val height=toolGridHeight(createPages[pager.currentPage],maxWidth-16.dp)+chrome
    SideEffect {preferred?.invoke(height)}
    Column(Modifier.height(height).padding(start=8.dp,end=8.dp,top=8.dp),verticalArrangement=Arrangement.spacedBy(4.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically) {
            Symbol("chevron_left","Back to attachments",back)
            Text("Create",Modifier.weight(1f),style=MaterialTheme.typography.titleMedium)
            Symbol("help","Help") {open("Help")}
        }
        HorizontalPager(pager,Modifier.weight(1f).fillMaxWidth().testTag("create-pages"),verticalAlignment=Alignment.Top) {page->
            LazyVerticalGrid(GridCells.Fixed(4),Modifier.fillMaxSize(),
                verticalArrangement=Arrangement.spacedBy(4.dp),horizontalArrangement=Arrangement.spacedBy(8.dp)) {
                items(createPages[page],key={it.first}) {(name,icon)->ComposerTool(name,icon) {open(name)}}
            }
        }
        Row(Modifier.align(Alignment.CenterHorizontally)) {
            repeat(createPages.size) {page->
                val active=pager.currentPage==page
                val dot by animateDpAsState(if(active)8.dp else 6.dp,motionPolicy.tween(MotionMillis),label="Create page dot")
                val tint by animateColorAsState(if(active)MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,motionPolicy.tween(MotionMillis),label="Create page dot tint")
                Box(Modifier.sizeIn(minWidth=48.dp,minHeight=48.dp).clip(RoundedCornerShape(12.dp))
                    .clickable(role=Role.Tab){scope.launch {if(motionPolicy.reduced)pager.scrollToPage(page) else pager.animateScrollToPage(page,animationSpec=motionPolicy.tween(MotionMillis))}}
                    .semantics {contentDescription="Create page ${page+1}";selected=active},contentAlignment=Alignment.Center) {
                    Box(Modifier.size(dot).background(tint,CircleShape))
                }
            }
        }
    }
}
}
