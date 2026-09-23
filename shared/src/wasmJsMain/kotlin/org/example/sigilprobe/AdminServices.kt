package org.sigil

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable

@Composable
internal fun AdminServices() {
    Text("Reference services",style=MaterialTheme.typography.headlineMedium)
    ServiceSettings(read={api("/admin/v0/services")},save={api("/admin/v0/services","PUT",it)},
        field={label,value,change,secret,enabled->Field(label,value,change,secret,enabled)})
}
