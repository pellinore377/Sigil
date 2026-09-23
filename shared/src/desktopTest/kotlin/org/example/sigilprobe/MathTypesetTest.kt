package org.sigil

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.toAwtImage
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.*

// The shared renderer Android and web both draw; SIGIL_MATH_RENDERS=<dir> saves the sheets.
class MathTypesetTest {
    @get:Rule val ui=createComposeRule()

    private val sources=listOf(
        "math::block\n\\int_0^\\infty e^{-x^2}\\,dx = \\frac{\\sqrt{\\pi}}{2};",
        "Energy: math::E = mc^2; and math::\\alpha_n^2; inline",
        "math::block\n\\begin{pmatrix}1 & 2 \\\\ 3 & 4\\end{pmatrix};",
        "math::block\nx = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a};",
        "math::block\n\\sum_{n=1}^{\\infty} \\frac{1}{n^2} = \\frac{\\pi^2}{6};",
        "math::block\n\\sqrt[3]{x} + x_i^2 + \\hat{y} + \\overline{AB} + \\lim_{x \\to 0} \\frac{\\sin x}{x};",
        "math::block\n\\alpha\\beta\\gamma\\delta\\epsilon\\theta\\lambda\\mu\\pi\\sigma\\phi\\omega\\,\\Gamma\\Delta\\Theta\\Lambda\\Sigma\\Omega;",
        "math::block\n(a_1+a_2+a_3+a_4+a_5+a_6+a_7+a_8+a_9+a_{10})^2 = \\sum_{i=1}^{10}\\sum_{j=1}^{10} a_i a_j;",
        "math::block\nf(x) = \\begin{cases} x^2 & x \\ge 0 \\\\ -x & \\text{otherwise}\\end{cases};",
        "math::block\n\\color{black}{x^2} + \\color{navy}{y^2} + \\color{red}{z};",
    )

    private fun message(source:String,mine:Boolean):ChatMessage {
        val part=ContentDecoder.part(NativeCore.structuredPreview(source))
        val parts=part.previewParts.ifEmpty {listOf(part)}
        return ChatMessage("m","author","",mine,"9:41","read",false,emptyList(),emptyList(),null,true,peer="c",parts=parts)
    }

    @Test fun every_example_is_typeset_from_the_font_not_shown_as_source() {
        sources.forEach {source->
            val parts=message(source,false).parts
            val formulas=parts.mapNotNull {it.utility}.filter {it.kind=="math"}
            assertTrue(formulas.isNotEmpty(),source)
            formulas.forEach {assertNotNull(it.math,"untypeset: ${it.display}");assertTrue(it.math!!.runs.isNotEmpty())}
        }
    }

    // Close-ups at a phone's density for review; skipped unless a directory is given.
    @Test fun close_ups() {
        val dir=System.getenv("SIGIL_MATH_RENDERS")?.let {java.io.File(it).apply {mkdirs()}} ?: return
        for(dark in listOf(false,true)) sources.forEachIndexed {i,source->
            ui.setContent {
                CompositionLocalProvider(LocalDensity provides Density(2.75f)) {
                    SigilTheme(Appearance(mode=if(dark)"Dark" else "Light"),palette=NativeCore::palette) {
                        val ground=MaterialTheme.colorScheme.surfaceContainerHigh
                        Box(Modifier.background(ground).padding(horizontal=14.dp,vertical=10.dp).width(320.dp)) {
                            CompositionLocalProvider(LocalContentColor provides MaterialTheme.colorScheme.onSurface,LocalMessageSurface provides ground) {MessageCards(message(source,false),{""},null)}
                        }
                    }
                }
            }
            javax.imageio.ImageIO.write(ui.onRoot().captureToImage().toAwtImage(),"png",java.io.File(dir,"close-${if(dark)"dark-" else ""}$i.png"))
        }
    }

    @Test fun sender_colours_stay_legible_on_either_bubble() {
        fun contrast(a:Color,b:Color)=(maxOf(a.luminance(),b.luminance())+.05f)/(minOf(a.luminance(),b.luminance())+.05f)
        val dark=Color(0xff2a2a2e); val light=Color(0xfff0f0f2)
        for((ground,ink) in listOf(dark to Color.White,light to Color.Black)) for(sent in listOf(Color.Black,Color(0xff000080),Color.White,Color.Yellow,Color.Red)) {
            assertTrue(contrast(legibleMathColor(sent,ground,ink),ground)>=3f,"$sent on $ground")
        }
        // A colour that already reads is left exactly as sent.
        assertEquals(Color.Red,legibleMathColor(Color.Red,light,Color.Black))
        // The script this font lacks shows the TeX source instead of replacement boxes.
        assertNull(message("math::block\n\\text{面积} = \\pi r^2;",false).parts.mapNotNull {it.utility}.single().math)
    }

    @Test fun formulas_draw_ink_in_all_four_bubbles() {
        val dir=System.getenv("SIGIL_MATH_RENDERS")?.let {java.io.File(it).apply {mkdirs()}}
        for(dark in listOf(false,true)) for(mine in listOf(false,true)) {
            ui.setContent {
                SigilTheme(Appearance(mode=if(dark)"Dark" else "Light"),palette=NativeCore::palette) {
                    val scheme=MaterialTheme.colorScheme
                    val ground=if(mine) (if(dark)Color(0xffd6d6d6) else Color(0xff484848)) else scheme.surfaceContainerHigh
                    val ink=if(mine) (if(dark)Color.Black else Color.White) else scheme.onSurface
                    Column(Modifier.background(scheme.surface).padding(16.dp).width(360.dp),verticalArrangement=Arrangement.spacedBy(12.dp)) {
                        sources.forEach {source->
                            Box(Modifier.background(ground,RoundedCornerShape(20.dp)).padding(horizontal=14.dp,vertical=10.dp)) {
                                CompositionLocalProvider(LocalContentColor provides ink,LocalMessageSurface provides ground) {MessageCards(message(source,mine),{""},null)}
                            }
                        }
                    }
                }
            }
            val sheet=ui.onRoot().captureToImage()
            dir?.let {javax.imageio.ImageIO.write(sheet.toAwtImage(),"png",java.io.File(it,"math-${if(dark)"dark" else "light"}-${if(mine)"outgoing" else "incoming"}.png"))}
            val formula=ui.onAllNodesWithContentDescription("Formula.",substring=true).fetchSemanticsNodes()
            assertTrue(formula.size>=sources.size)
            // The Gaussian integral's own box carries drawn ink, not an empty canvas.
            val image=ui.onAllNodesWithContentDescription("Formula.",substring=true)[0].captureToImage().toPixelMap()
            val corner=image[0,0]
            val inked=(0 until image.width).sumOf {x->(0 until image.height).count {y->image[x,y]!=corner}}
            assertTrue(inked>200,"ink pixels: $inked")
            ui.onNodeWithText("\\frac{\\sqrt{\\pi}}{2}",substring=true).assertDoesNotExist()
        }
    }
}
