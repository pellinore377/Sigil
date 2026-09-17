package org.sigil

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.shape.RoundedCornerShape

data class OperationalSample(
    val at: Long,val accounts: Long,val devices: Long,val messages: Long,val federation: Long,val push: Long,
    val failedPeers: Long,val pushRetries: Long,val invalidPush: Long,val database: Long,val attachments: Long,val recovery: Long,
    val maintenance: Long,val failedMaintenance: Long,val lastBackup: Long?,val restorePending: Boolean,val version: String,val schema: Long,
)
internal fun observationWindow(history: List<OperationalSample>,sample: OperationalSample): List<OperationalSample> =
    (history.filter { it.at<sample.at && it.at>=sample.at-1800 }+sample).takeLast(120)

internal fun storageSize(bytes: Long): String {
    val unit=when { bytes>=1073741824L -> 1073741824L to "GiB";bytes>=1048576 -> 1048576L to "MiB";bytes>=1024 -> 1024L to "KiB";else -> 1L to "B" }
    return if(unit.first==1L)"$bytes B" else "${bytes/unit.first}.${(bytes%unit.first)*10/unit.first} ${unit.second}"
}
@OptIn(kotlin.time.ExperimentalTime::class)
internal fun observationTime(seconds: Long)=kotlin.time.Instant.fromEpochSeconds(seconds).toString().replace('T',' ').replace("Z"," UTC")

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun OperationalDashboard(samples: List<OperationalSample>,loading: Boolean,error: String,stale: Boolean,refresh: () -> Unit,navigate: (String) -> Unit) {
    val current=samples.lastOrNull()
    Column(Modifier.fillMaxSize(),horizontalAlignment=Alignment.CenterHorizontally) {
    Column(Modifier.widthIn(max=680.dp).fillMaxWidth().padding(horizontal=16.dp),verticalArrangement=Arrangement.spacedBy(24.dp)) {
        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("Overview",style=MaterialTheme.typography.headlineSmall)
                Text(current?.let { "Observed ${observationTime(it.at)}" } ?: "Waiting for the first observation",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
            SigilIconButton(refresh,enabled=!loading) { Glyph("refresh",24,"Refresh server overview") }
        }
        if(error.isNotEmpty()) Text(error,color=MaterialTheme.colorScheme.error,modifier=Modifier.semantics { liveRegion=LiveRegionMode.Polite })
        if(stale && current!=null) Text("These observations are stale. Refresh to check the server.",color=MaterialTheme.colorScheme.error)
        if(current==null) {
            if(loading) LinearProgressIndicator(Modifier.fillMaxWidth())
            Box(Modifier.fillMaxWidth().padding(32.dp),contentAlignment=Alignment.Center) { Text("No operational history yet.",color=MaterialTheme.colorScheme.onSurfaceVariant) }
        } else {
            BoxWithConstraints(Modifier.fillMaxWidth()) {
                val narrow=maxWidth<640.dp
                val metrics: @Composable (Modifier)->Unit = { modifier ->
                    OperationalMetric("Accounts",current.accounts.toString(),"People", {navigate("Users")},modifier,narrow)
                    OperationalMetric("Devices",current.devices.toString(),"People", {navigate("Users")},modifier,narrow)
                    OperationalMetric("Database",storageSize(current.database),"Server", {navigate("Server")},modifier,narrow)
                }
                if(narrow) Column(verticalArrangement=Arrangement.spacedBy(12.dp)) {metrics(Modifier.fillMaxWidth())}
                else Row(horizontalArrangement=Arrangement.spacedBy(16.dp)) {metrics(Modifier.weight(1f))}
            }
            DashboardSection("Delivery queues") {
                Text("Queued items include offline recipients.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                QueueHistory(samples)
            }
            DashboardSection("Encrypted payload storage") {
                val total=current.attachments.toDouble()+current.recovery.toDouble()
                val filled=MaterialTheme.colorScheme.primary
                val remainder=LocalContentColor.current.copy(alpha=.13f)
                if(total>0) Canvas(Modifier.fillMaxWidth().height(24.dp).clip(RoundedCornerShape(12.dp)).semantics { contentDescription="Encrypted attachment and recovery payload proportions" }) {
                    val split=(current.attachments.toDouble()/total*size.width).toFloat()
                    drawRect(filled,size=androidx.compose.ui.geometry.Size(split,size.height))
                    drawRect(remainder,topLeft=Offset(split,0f),size=androidx.compose.ui.geometry.Size(size.width-split,size.height))
                }
                MetricLine("Attachments",storageSize(current.attachments))
                MetricLine("Recovery",storageSize(current.recovery))
                Text("Included in database allocation. Free disk space and journals are not measured.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                SigilTextButton({navigate("Server")}) { Text("Server settings") }
            }
            DashboardSection("Attention and maintenance") {
                val warnings=current.failedPeers>0 || current.invalidPush>0 || current.failedMaintenance>0 || current.restorePending
                Text(if(warnings)"Some services need attention." else "No failures reported in this observation.",color=if(warnings)MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface)
                MetricLine("Federation peers reporting errors",current.failedPeers.toString())
                MetricLine("Push channels requiring registration",current.invalidPush.toString())
                MetricLine("Attempts on queued notifications",current.pushRetries.toString())
                MetricLine("Maintenance queued or running",current.maintenance.toString())
                MetricLine("Failed maintenance operations",current.failedMaintenance.toString())
                Text(if(current.restorePending)"A restore is waiting for restart." else "No restore is waiting for restart.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
                Text(current.lastBackup?.let { "Last completed backup, restore or upgrade: ${observationTime(it)}" } ?: "No completed backup, restore or upgrade recorded.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text("Server ${current.version} · schema ${current.schema}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
    }
}

@Composable
private fun OperationalMetric(label: String,value: String,destination: String,open: () -> Unit,modifier: Modifier,compact: Boolean) {
    Surface(modifier.clickable(role=Role.Button,onClickLabel="Open $destination",onClick=open),shape=RoundedCornerShape(18.dp),color=MaterialTheme.colorScheme.surfaceContainerHigh) {
        if(compact) Row(Modifier.padding(20.dp),verticalAlignment=Alignment.CenterVertically,horizontalArrangement=Arrangement.spacedBy(16.dp)) {
            Text(label,Modifier.weight(1f),style=MaterialTheme.typography.labelLarge,color=MaterialTheme.colorScheme.onSurfaceVariant)
            Text(value,style=MaterialTheme.typography.headlineSmall)
        } else Column(Modifier.padding(24.dp),verticalArrangement=Arrangement.spacedBy(8.dp)) { Text(label,style=MaterialTheme.typography.labelLarge,color=MaterialTheme.colorScheme.onSurfaceVariant);Text(value,style=MaterialTheme.typography.headlineSmall) }
    }
}
@Composable
private fun DashboardSection(title: String,content: @Composable ColumnScope.() -> Unit) {
    Column(Modifier.fillMaxWidth(),verticalArrangement=Arrangement.spacedBy(12.dp)) {
        Text(title,Modifier.padding(start=12.dp,bottom=8.dp),style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant)
        content()
    }
}
@Composable
private fun MetricLine(label: String,value: String) {
    Row(Modifier.fillMaxWidth().padding(horizontal=12.dp),horizontalArrangement=Arrangement.spacedBy(12.dp)) {
        Text(label,Modifier.weight(1f),style=MaterialTheme.typography.bodyMedium,color=MaterialTheme.colorScheme.onSurfaceVariant,maxLines=2,overflow=androidx.compose.ui.text.style.TextOverflow.Ellipsis)
        Text(value,style=MaterialTheme.typography.titleMedium)
    }
}
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun QueueHistory(samples: List<OperationalSample>) {
    var selected by remember { mutableStateOf<Long?>(null) }
    var values by remember { mutableStateOf(false) }
    val current=samples.firstOrNull { it.at==selected } ?: samples.last()
    val series=listOf("Messages" to { v:OperationalSample -> v.messages },"Federation" to { v:OperationalSample -> v.federation },"Notifications" to { v:OperationalSample -> v.push })
    val ink=listOf(MaterialTheme.colorScheme.onSurface,MaterialTheme.colorScheme.onSurfaceVariant,MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha=.55f))
    var enabled by remember { mutableStateOf(setOf(0,1,2)) }
    FlowRow(horizontalArrangement=Arrangement.spacedBy(8.dp)) {
        series.forEachIndexed { index,(name,_) ->
            SigilTextButton({enabled=if(index in enabled)enabled-index else enabled+index},modifier=Modifier.semantics { role=Role.Checkbox;stateDescription=if(index in enabled)"Shown" else "Hidden" }) {
                Glyph(if(index in enabled)"check_circle" else "circle",18,filled=index in enabled);Spacer(Modifier.width(6.dp));Text("$name · ${listOf("solid","dashed","dotted")[index]}")
            }
        }
    }
    if(samples.size<2) Box(Modifier.fillMaxWidth().padding(32.dp),contentAlignment=Alignment.Center) { Text("Collecting history. The first observation is shown below.",color=MaterialTheme.colorScheme.onSurfaceVariant) }
    else {
        val max=series.indices.filter { it in enabled }.maxOfOrNull { i -> samples.maxOf { series[i].second(it) }.toDouble() }?.coerceAtLeast(1.0) ?: 1.0
        Text("0–${max.toLong()} waiting items",style=MaterialTheme.typography.labelMedium)
        Canvas(Modifier.fillMaxWidth().height(180.dp).semantics { contentDescription="Delivery queue history. Numeric observations and series controls follow." }) {
            val start=samples.first().at;val duration=(samples.last().at-start).coerceAtLeast(1).toDouble()
            series.forEachIndexed { index,(_,value) -> if(index in enabled) {
                fun point(sample:OperationalSample)=Offset(((sample.at-start)/duration*size.width).toFloat(),(size.height-value(sample)/max*size.height).toFloat())
                val pattern=when(index) {1 -> androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(8.dp.toPx(),5.dp.toPx()));2 -> androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(2.dp.toPx(),5.dp.toPx()));else -> null}
                samples.zipWithNext().forEach { (a,b) -> if(b.at-a.at<=45) drawLine(ink[index],point(a),point(b),2.dp.toPx(),pathEffect=pattern) }
                samples.forEach { drawCircle(ink[index],if(it.at==current.at)4.dp.toPx() else 2.dp.toPx(),point(it)) }
            } }
        }
        Text("${observationTime(samples.first().at)} — ${observationTime(samples.last().at)}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
        Text("Gaps longer than 45 seconds are left disconnected. History stays in this dashboard session.",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant)
    }
    series.forEach { (name,value) -> MetricLine(name,value(current).toString()) }
    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
        val index=samples.indexOf(current)
        SigilIconButton({selected=samples[index-1].at},enabled=index>0) { Glyph("chevron_left",24,"Previous observation") }
        Text(observationTime(current.at),Modifier.weight(1f),style=MaterialTheme.typography.labelMedium)
        SigilIconButton({selected=samples[index+1].at},enabled=index<samples.lastIndex) { Glyph("chevron_right",24,"Next observation") }
    }
    SigilTextButton({values=!values}) { Text(if(values)"Hide observations" else "Show all observations") }
    Expandable(values) {
        samples.asReversed().forEach { sample -> Text("${observationTime(sample.at)} · Messages ${sample.messages} · Federation ${sample.federation} · Notifications ${sample.push}",style=MaterialTheme.typography.bodySmall,color=MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}
