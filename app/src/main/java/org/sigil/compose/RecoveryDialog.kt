package org.sigil.compose

import android.content.ClipData
import android.content.ClipboardManager
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.SecureFlagPolicy

@Composable
internal fun RecoveryDialog(secret:String,busy:Boolean,dismiss:()->Unit,enable:()->Unit) {
    val context=LocalContext.current
    org.sigil.RecoverySetup(secret,busy,dismiss,enable,DialogProperties(securePolicy=SecureFlagPolicy.SecureOn)) {
        val clipboard=context.getSystemService(ClipboardManager::class.java)
        val clip=ClipData.newPlainText("Sigil recovery key",secret)
        clip.description.extras=PersistableBundle().apply {putBoolean("android.content.extra.IS_SENSITIVE",true)}
        clipboard.setPrimaryClip(clip)
        Handler(Looper.getMainLooper()).postDelayed({if(clipboard.primaryClip?.getItemAt(0)?.text?.toString()==secret)clipboard.clearPrimaryClip()},60000)
    }
}
@Composable
internal fun RestoreRecoveryDialog(busy:Boolean,issue:String?,dismiss:()->Unit,restore:(String)->Unit)=
    org.sigil.RecoveryRestore(busy,issue,dismiss,restore,DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))
@Composable
internal fun AccountRecoveryDialog(sso:Boolean,busy:Boolean,issue:String?,dismiss:()->Unit,recover:(String,String?)->Unit)=
    org.sigil.AccountRecovery(sso,busy,issue,dismiss,recover,DialogProperties(securePolicy=SecureFlagPolicy.SecureOn))
