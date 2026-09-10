package org.sigil

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toPixelMap
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import org.junit.Rule
import org.junit.Test
import kotlin.test.assertTrue

class CardActionTest {
    @get:Rule val ui=createComposeRule()
    @Test fun actions_remain_visible_when_the_card_matches_the_theme_primary() {
        val gray=Color(0xff555555)
        ui.setContent {MaterialTheme(colorScheme=lightColorScheme(primary=gray)) {
            Surface(color=gray,contentColor=Color.White) {
                CompositionLocalProvider(LocalMessageSurface provides gray) {
                    SigilTextButton({},Modifier.width(180.dp).testTag("action")) {Text("Open dice")}
                }
            }
        }}
        val pixels=ui.onNodeWithTag("action").captureToImage().toPixelMap()
        var visible=0
        repeat(pixels.height) {y->repeat(pixels.width) {x->
            val pixel=pixels[x,y]
            if(pixel.red>.8f && pixel.green>.8f && pixel.blue>.8f)visible++
        }}
        assertTrue(visible>20,"The action must contrast with its outgoing card")
    }
}
