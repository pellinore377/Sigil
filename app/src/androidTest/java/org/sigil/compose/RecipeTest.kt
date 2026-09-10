package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class RecipeTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()
    @Test fun cooking_keeps_checks_local_preserves_steps_and_retries_serving_changes() {
        val recipe = RecipeContent(RichText("Synthetic dinner"), 4, 4, 1500, listOf(RichText("200g flour"), RichText("1-2 eggs")), listOf(false, false), listOf(RichText("Combine the flour and eggs."), RichText("Bake until ready.")))
        val chat = ChatSummary("self", "@sam:example.test", "", "", true, emptyList())
        val message = ChatMessage("recipe", "sam", "Recipe", true, "9:33", "sent", false, emptyList(), emptyList(), null, true,
            timestamp = 1000, parts = listOf(MessagePart("card", "recipe", "Recipe", recipe = recipe)))
        var changes = 0
        val commands = mutableListOf<String>()
        ui.runOnUiThread { ui.activity.setSigilContent {
            CompositionLocalProvider(LocalRecipeScale provides { _, _, serves ->
                changes++
                if (changes == 1) throw java.io.IOException("Synthetic local failure")
                assertEquals(5, serves)
                recipe.copy(serves = 5, ingredients = listOf(RichText("250g flour"), RichText("1-2 eggs")), scaled = listOf(true, false))
            }) {
                SigilApp(NativeCore::palette, NativeCore::analyze, MessengerState(phase = "connected", chats = listOf(chat), selected = "self", messages = listOf(message)), { name, _ -> commands += name })
            }
        } }
        ui.onNodeWithText("Open recipe").performClick()
        ui.onNode(hasText("200g flour") and hasAnyAncestor(isDialog())).performClick().assertIsOn()
        ui.onNodeWithText("Steps").performClick()
        ui.onNodeWithText("Step 1 of 2").assertIsDisplayed()
        ui.onNodeWithText("Next").performClick()
        ui.onNodeWithText("Bake until ready.").assertIsDisplayed()
        ui.onNodeWithText("Next").assertIsNotEnabled()
        ui.onNodeWithText("Ingredients").performClick()
        ui.onNode(hasText("200g flour") and hasAnyAncestor(isDialog())).assertIsOn()
        ui.onNodeWithContentDescription("More servings").performClick()
        ui.onNodeWithText("Couldn't adjust servings. The previous amounts are still shown.").assertIsDisplayed()
        ui.onNodeWithContentDescription("More servings").performClick()
        ui.onNode(hasText("250g flour") and hasAnyAncestor(isDialog())).assertIsOn()
        ui.onNodeWithText("Serves 5 · 25 min").assertIsDisplayed()
        ui.onNodeWithText("Keep screen awake").performClick().assertIsOn()
        fun awake(view: android.view.View): Boolean = view.keepScreenOn || view is android.view.ViewGroup && (0 until view.childCount).any { awake(view.getChildAt(it)) }
        ui.runOnIdle { assertTrue(android.view.inspector.WindowInspector.getGlobalWindowViews().any(::awake)) }
        ui.onNodeWithContentDescription("Close recipe").performClick()
        ui.runOnIdle { assertFalse(android.view.inspector.WindowInspector.getGlobalWindowViews().any(::awake)); assertTrue(commands.none { it in listOf("card_action", "post", "edit") }) }
        ui.onNodeWithText("200g flour").assertIsDisplayed()
    }
}
