package org.sigil.compose

import androidx.activity.ComponentActivity
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.sigil.*

class DashboardTest {
    @get:Rule val ui=createAndroidComposeRule<ComponentActivity>()
    @Test fun observations_have_numeric_navigation_empty_history_and_actionable_destinations() {
        val samples=mutableStateOf(emptyList<OperationalSample>())
        var destination=""
        ui.setContent { MaterialTheme { Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
            OperationalDashboard(samples.value,false,"",false,{},{destination=it})
        } } }
        ui.onNodeWithText("No operational history yet.").assertIsDisplayed()
        val first=OperationalSample(1800000000,2,3,5,2,1,0,0,0,1048576,20,10,0,0,null,false,"test",33)
        ui.runOnIdle { samples.value=listOf(first) }
        ui.onNodeWithText("Active accounts").performClick()
        assertEquals("Users",destination)
        ui.onNodeWithText("Collecting history. The first observation is shown below.").performScrollTo().assertIsDisplayed()
        ui.runOnIdle { samples.value=listOf(first,first.copy(at=1800000015,messages=0,push=0)) }
        ui.onNodeWithContentDescription("Previous observation").performScrollTo().performClick()
        ui.onNodeWithContentDescription("Next observation").assertIsEnabled().performClick()
        ui.onNodeWithContentDescription("Next observation").assertIsNotEnabled()
        ui.onNodeWithText("Show all observations").performScrollTo().performClick()
        ui.onNodeWithText("2027-01-15 08:00:00 UTC · Messages 5 · Federation 2 · Notifications 1").performScrollTo().assertIsDisplayed()
        ui.onNodeWithText("Hide observations").performScrollTo().performClick()
        ui.onNodeWithText("No failures reported in this observation.").performScrollTo().assertIsDisplayed()
    }
}
