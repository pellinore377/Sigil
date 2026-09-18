package org.sigil

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal val AttachmentCardWidth = 300.dp
private val PageHeight = 176.dp
private val PagePaper = Color(0xfff6f5f2)
private val PageInk = Color(0xff1c1b1a)

// The reference's type badge: a dark pill in the page's bottom corner.
@Composable private fun KindChip(kind: AttachmentKind, modifier: Modifier = Modifier) {
    Text(kind.chip, modifier.background(Color(0xff3a3a3a), RoundedCornerShape(6.dp)).padding(horizontal = 7.dp, vertical = 3.dp),
        color = Color.White, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, letterSpacing = .6.sp, maxLines = 1)
}

// A document arrives as the top of its own first page, cut by a fade and stamped with its type.
@Composable fun DocumentCard(name: String, kind: AttachmentKind, bytes: Long, peek: FilePeek?, loading: Boolean, shape: Shape, onOpen: () -> Unit) {
    Box(Modifier.widthIn(max = AttachmentCardWidth).fillMaxWidth().height(PageHeight).clip(shape).background(PagePaper).clickable(onClick = onOpen).semanticsLabel("$name, ${kind.chip} ${attachmentSize(bytes)}")) {
        when (peek) {
            is FilePeek.Text -> Text(if (peek.markdown) markdownPreview(peek.text) else androidx.compose.ui.text.AnnotatedString(peek.text),
                Modifier.fillMaxSize().padding(horizontal = 16.dp, vertical = 14.dp).clipToBounds(), color = PageInk, fontSize = 11.sp, lineHeight = 15.sp, overflow = TextOverflow.Clip)
            is FilePeek.Table -> SheetGrid(peek.cells, Modifier.fillMaxSize().padding(10.dp))
            is FilePeek.Page -> Image(peek.image, null, Modifier.fillMaxSize(), contentScale = ContentScale.FillWidth, alignment = Alignment.TopCenter)
            else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (loading) CircularProgressIndicator(Modifier.size(28.dp), color = PageInk.copy(alpha = .5f), strokeWidth = 2.dp)
                else CompositionLocalProvider(LocalContentColor provides PageInk.copy(alpha = .45f)) { Glyph(kind.glyph, 40) }
            }
        }
        // The page runs on past the card; the fade says so without a hard edge.
        Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(56.dp).background(Brush.verticalGradient(listOf(Color.Transparent, PagePaper))))
        KindChip(kind, Modifier.align(Alignment.BottomStart).padding(10.dp))
    }
}

// The reference's grid: header row on grey, ruled cells, everything clipped to the page.
@Composable private fun SheetGrid(cells: List<List<String>>, modifier: Modifier) {
    val columns = cells.maxOfOrNull { it.size } ?: 0
    if (columns == 0) return
    val rule = PageInk.copy(alpha = .14f)
    Column(modifier.clipToBounds()) {
        cells.take(12).forEachIndexed { r, row ->
            Row(Modifier.fillMaxWidth().then(if (r == 0) Modifier.background(PageInk.copy(alpha = .06f)) else Modifier)) {
                for (c in 0 until minOf(columns, 6)) {
                    Text(row.getOrNull(c).orEmpty(), Modifier.weight(if (c == 1) 1.6f else 1f).border(.5.dp, rule).padding(horizontal = 5.dp, vertical = 3.dp),
                        color = PageInk, fontSize = 9.sp, lineHeight = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, fontWeight = if (r == 0) FontWeight.SemiBold else FontWeight.Normal)
                }
            }
        }
    }
}

// A track is a square of its own artwork, or of the bubble's tone, with the title strip along the bottom.
@Composable fun AudioCard(title: String, detail: String, art: ImageBitmap?, tint: Color?, shape: Shape, onOpen: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val ground = tint ?: scheme.surfaceContainerHigh
    val ink = if (ground.brightness() > .5f) Color(0xff1c1b1a) else Color.White
    val strip = lerp(ground, if (ink == Color.White) Color.Black else Color.White, .22f)
    Box(Modifier.widthIn(max = AttachmentCardWidth).fillMaxWidth().aspectRatio(1f).clip(shape).background(ground).clickable(onClick = onOpen).semanticsLabel("$title, $detail")) {
        if (art != null) Image(art, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        Box(Modifier.align(Alignment.Center).size(64.dp).background(if (art != null) Color.Black.copy(alpha = .5f) else lerp(ground, ink, .2f), SquircleShape), contentAlignment = Alignment.Center) {
            Text("\u266b", color = if (art != null) Color.White else ink, fontSize = 30.sp, lineHeight = 30.sp)
        }
        Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().background(strip.copy(alpha = if (art != null) .82f else 1f)).padding(horizontal = 14.dp, vertical = 10.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(title, color = ink, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(detail, color = ink.copy(alpha = .8f), style = MaterialTheme.typography.labelSmall, maxLines = 1)
        }
    }
}

// Anything without a picture of itself: the reference's icon, name and size on one line.
internal val SquircleShape = androidx.compose.foundation.shape.GenericShape { size, _ -> addPath(squirclePath(androidx.compose.ui.geometry.Rect(androidx.compose.ui.geometry.Offset.Zero, size), size.minDimension * .42f)) }
@Composable fun FileChip(name: String, bytes: Long, progress: Boolean, onOpen: () -> Unit) {
    Row(Modifier.widthIn(max = AttachmentCardWidth).clickable(onClick = onOpen).padding(start = 4.dp, end = 18.dp, top = 10.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(16.dp)) {
        Box(Modifier.size(46.dp).background(LocalContentColor.current.copy(alpha = .12f), SquircleShape), contentAlignment = Alignment.Center) {
            if (progress) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = LocalContentColor.current) else Glyph("draft", 22)
        }
        Column(Modifier.weight(1f, fill = false), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            Text(name, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            Text(attachmentSize(bytes), style = MaterialTheme.typography.labelSmall, color = LocalContentColor.current.copy(alpha = .7f), maxLines = 1)
        }
    }
}

private fun Modifier.semanticsLabel(label: String) = semantics { contentDescription = label }
