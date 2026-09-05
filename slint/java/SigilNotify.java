// The notification shade, as Google Messages draws it: one notification per
// conversation, the last few messages of the thread inside it, a Reply field
// that sends without opening the app, and a Mark as read that clears it.
//
// WHAT DECIDES WHAT, AND WHERE. Nothing here decides whether a message is
// worth a notification — that is the account's notify settings, the room's
// mode, whose message it is and which room is on screen, and all of that is
// known on the Rust side (bridge.rs). Java is told to post and it posts.
//
// WHY MessagingStyle. It is the only style the platform treats as a
// conversation: the shade groups it under Conversations, gives it the
// sender's face per line, and lets the reply field stand under it. It wants
// the WHOLE recent thread on every post rather than only the new line, so the
// last eight messages per room are kept in `ROOMS` here — the alternative is
// asking the engine for a timeline from a broadcast receiver, on a process
// that may have no engine at all.
//
// android.app, not androidx: this app has no AndroidX on its class path (see
// build.rs — one javac, one d8, no Gradle), so every class named here is a
// framework class and everything newer than min_sdk 26 is behind an SDK_INT
// check.
//
// Lives in the APK's classes.dex, beside SigilReceiver and SigilService,
// because a manifest component is loaded by the app's own class loader and
// the receiver and this file must share the same `ROOMS`.

package com.sigil.slint;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Person;
import android.app.RemoteInput;
import android.content.Context;
import android.content.Intent;
import android.content.LocusId;
import android.content.pm.ShortcutInfo;
import android.content.pm.ShortcutManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.PorterDuff;
import android.graphics.PorterDuffXfermode;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.drawable.Icon;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;

import java.io.File;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;

public final class SigilNotify {
    private SigilNotify() {}

    static final String TAG = "SigilNotify";

    /// The conversations channel: importance HIGH, so a message arrives with
    /// sound and a heads-up the way the platform messenger's does.
    public static final String CH_MESSAGES = "messages";
    /// The foreground service's channel: importance MIN, silent, no badge.
    /// It exists because a foreground service must show something, not
    /// because anyone wants to see it.
    public static final String CH_CONNECTION = "connection";

    /// Every conversation notification is grouped under this key, so the
    /// shade stacks them behind one summary rather than listing eight rooms.
    static final String GROUP = "sigil-messages";
    static final String SUMMARY_TAG = "sigil-summary";
    static final int SUMMARY_ID = 1;
    /// The foreground service's own notification id.
    public static final int SERVICE_ID = 2;

    /// The receiver's contract. `KEY_TEXT` is the RemoteInput key the reply
    /// field writes into.
    public static final String ACTION_REPLY = "com.sigil.slint.REPLY";
    public static final String ACTION_MARK_READ = "com.sigil.slint.MARK_READ";
    public static final String EXTRA_ROOM = "roomId";
    public static final String KEY_TEXT = "text";

    /// How much of a thread the shade shows. Eight is what MessagingStyle
    /// draws before it starts eliding, and more is only memory.
    static final int KEEP = 8;

    /// One line in a conversation. `sender` empty means it is ours — the
    /// reply that was just sent from the shade.
    static final class Msg {
        final String sender;
        final String avatar;
        final String text;
        final long ts;
        Msg(String sender, String avatar, String text, long ts) {
            this.sender = sender == null ? "" : sender;
            this.avatar = avatar == null ? "" : avatar;
            this.text = text == null ? "" : text;
            this.ts = ts;
        }
    }

    /// A conversation as the shade knows it. Kept because every post has to
    /// redraw the whole thread, and because the receiver's reply has to be
    /// appended to something.
    static final class Room {
        String name = "";
        boolean group;
        int unread;
        final ArrayList<Msg> msgs = new ArrayList<Msg>();
    }

    /// Insertion-ordered so the summary lists rooms as they arrived.
    static final LinkedHashMap<String, Room> ROOMS = new LinkedHashMap<String, Room>();

    /// Faces already decoded, by file path. A room's avatar is the same file
    /// on every message it sends.
    static final LinkedHashMap<String, Icon> FACES = new LinkedHashMap<String, Icon>();

    // ------------------------------------------------------------- channels

    /// Both channels, made once. Creating a channel that exists is a no-op,
    /// so this is called on every path that posts anything.
    public static synchronized void ensureChannels(Context c) {
        NotificationManager nm = manager(c);
        if (nm == null) return;
        try {
            NotificationChannel msgs = new NotificationChannel(
                    CH_MESSAGES, "Messages", NotificationManager.IMPORTANCE_HIGH);
            msgs.setDescription("New messages");
            msgs.enableVibration(true);
            msgs.enableLights(true);
            msgs.setShowBadge(true);
            msgs.setLockscreenVisibility(Notification.VISIBILITY_PRIVATE);
            // A conversation notification may bubble; the person decides in
            // the shade's own settings, this only allows it.
            if (Build.VERSION.SDK_INT >= 29) msgs.setAllowBubbles(true);
            nm.createNotificationChannel(msgs);

            NotificationChannel conn = new NotificationChannel(
                    CH_CONNECTION, "Connection", NotificationManager.IMPORTANCE_MIN);
            conn.setDescription("Sigil stays connected while it is in the background");
            conn.setSound(null, null);
            conn.enableVibration(false);
            conn.enableLights(false);
            conn.setShowBadge(false);
            nm.createNotificationChannel(conn);
        } catch (Throwable t) {
            Log.w(TAG, "channels: " + t);
        }
    }

    // ---------------------------------------------------------------- post

    /// One incoming message in `roomId`. The room's thread grows by this
    /// line and the whole notification is redrawn from it.
    public static synchronized void post(Context c, String roomId, String roomName,
                                         boolean isGroup, String senderName,
                                         String senderAvatarPath, String text,
                                         long tsMs, int unread) {
        if (c == null || roomId == null || roomId.isEmpty()) return;
        try {
            ensureChannels(c);
            Room r = ROOMS.get(roomId);
            if (r == null) {
                r = new Room();
                ROOMS.put(roomId, r);
            }
            r.name = roomName == null ? "" : roomName;
            r.group = isGroup;
            r.unread = unread;
            add(r, new Msg(senderName == null || senderName.isEmpty() ? r.name : senderName,
                    senderAvatarPath, text, tsMs > 0 ? tsMs : System.currentTimeMillis()));
            show(c, roomId, r);
            summary(c);
        } catch (Throwable t) {
            Log.w(TAG, "post: " + t);
        }
    }

    /// The reply that was just sent from the shade, shown in the thread as
    /// ours. Google Messages does this the instant the arrow is tapped, and
    /// it is the only sign the person gets that the reply left the phone.
    public static synchronized void appendOwn(Context c, String roomId, String text) {
        if (c == null || roomId == null) return;
        Room r = ROOMS.get(roomId);
        if (r == null) return;
        try {
            add(r, new Msg("", "", text, System.currentTimeMillis()));
            r.unread = 0;
            show(c, roomId, r);
        } catch (Throwable t) {
            Log.w(TAG, "reply echo: " + t);
        }
    }

    private static void add(Room r, Msg m) {
        r.msgs.add(m);
        while (r.msgs.size() > KEEP) r.msgs.remove(0);
    }

    /// Build and post `roomId`'s notification from the thread we hold.
    @SuppressWarnings("deprecation")
    private static void show(Context c, String roomId, Room r) {
        NotificationManager nm = manager(c);
        if (nm == null || r.msgs.isEmpty()) return;
        int id = idOf(roomId);
        Msg last = r.msgs.get(r.msgs.size() - 1);

        Notification.Builder b = new Notification.Builder(c, CH_MESSAGES);
        b.setStyle(style(c, r));
        b.setSmallIcon(smallIcon(c));
        b.setContentTitle(r.name);
        b.setContentText(last.text);
        b.setContentIntent(openIntent(c, roomId, id));
        b.setAutoCancel(true);
        b.setCategory(Notification.CATEGORY_MESSAGE);
        // Deprecated since channels arrived, and still what a device below 26
        // — and some OEM shades above it — sorts by.
        b.setPriority(Notification.PRIORITY_HIGH);
        // Every message in a conversation rings: this is not a progress bar.
        b.setOnlyAlertOnce(false);
        b.setWhen(last.ts);
        b.setShowWhen(true);
        b.setGroup(GROUP);
        if (r.unread > 1) b.setNumber(r.unread);
        // Sound and vibration are the channel's from API 26 on, and this app
        // has no device below that; nothing is set here on purpose.
        b.setShortcutId(roomId);
        if (Build.VERSION.SDK_INT >= 29) b.setLocusId(new LocusId(roomId));
        if (Build.VERSION.SDK_INT >= 26) {
            Icon face = face(last.avatar);
            if (face != null) b.setLargeIcon(face);
        }
        b.addAction(replyAction(c, roomId, id));
        b.addAction(markReadAction(c, roomId, id));

        shortcut(c, roomId, r, last);
        nm.notify(roomId, id, b.build());
    }

    /// The thread, as MessagingStyle wants it: every line with the person who
    /// wrote it, ours with no person at all.
    @SuppressWarnings("deprecation")
    private static Notification.Style style(Context c, Room r) {
        if (Build.VERSION.SDK_INT >= 28) {
            Person me = new Person.Builder().setName("You").setKey("self").build();
            Notification.MessagingStyle s = new Notification.MessagingStyle(me);
            s.setGroupConversation(r.group);
            // The title is what the shade puts above a group's lines; a
            // one-to-one conversation is already titled by the person.
            if (r.group) s.setConversationTitle(r.name);
            for (int i = 0; i < r.msgs.size(); i++) {
                Msg m = r.msgs.get(i);
                Person who = m.sender.isEmpty() ? null : person(m);
                s.addMessage(new Notification.MessagingStyle.Message(m.text, m.ts, who));
            }
            return s;
        }
        // API 26–27: Person does not exist yet, and the sender is a string.
        Notification.MessagingStyle s = new Notification.MessagingStyle("You");
        if (r.group) s.setConversationTitle(r.name);
        for (int i = 0; i < r.msgs.size(); i++) {
            Msg m = r.msgs.get(i);
            s.addMessage(m.text, m.ts, m.sender.isEmpty() ? null : m.sender);
        }
        return s;
    }

    private static Person person(Msg m) {
        Person.Builder p = new Person.Builder().setName(m.sender).setKey(m.sender);
        Icon face = face(m.avatar);
        if (face != null) p.setIcon(face);
        return p.build();
    }

    // -------------------------------------------------------------- actions

    /// Reply: a RemoteInput on a broadcast to SigilReceiver. The component is
    /// named explicitly — an implicit broadcast would be refused on API 26+ —
    /// and the PendingIntent must be MUTABLE, because the RemoteInput's text
    /// is written INTO the intent by the shade before it is sent.
    private static Notification.Action replyAction(Context c, String roomId, int id) {
        RemoteInput input = new RemoteInput.Builder(KEY_TEXT)
                .setLabel("Reply")
                .setAllowFreeFormInput(true)
                .build();
        Intent i = new Intent(c, SigilReceiver.class)
                .setAction(ACTION_REPLY)
                .putExtra(EXTRA_ROOM, roomId);
        int flags = PendingIntent.FLAG_UPDATE_CURRENT
                | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
        PendingIntent pi = PendingIntent.getBroadcast(c, id, i, flags);
        Notification.Action.Builder a = new Notification.Action.Builder(
                Icon.createWithResource(c, android.R.drawable.ic_menu_send), "Reply", pi)
                .addRemoteInput(input)
                .setAllowGeneratedReplies(true);
        if (Build.VERSION.SDK_INT >= 28) {
            a.setSemanticAction(Notification.Action.SEMANTIC_ACTION_REPLY);
        }
        return a.build();
    }

    /// Mark as read: nothing is typed, so the PendingIntent is IMMUTABLE.
    private static Notification.Action markReadAction(Context c, String roomId, int id) {
        Intent i = new Intent(c, SigilReceiver.class)
                .setAction(ACTION_MARK_READ)
                .putExtra(EXTRA_ROOM, roomId);
        PendingIntent pi = PendingIntent.getBroadcast(c, id, i,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        // No setShowsUserInterface here: that one is AndroidX's, and the
        // framework builder has no such method. SEMANTIC_ACTION_MARK_AS_READ
        // is what tells the shade (and Android Auto) that this button answers
        // in place rather than opening anything.
        Notification.Action.Builder a = new Notification.Action.Builder(
                Icon.createWithResource(c, android.R.drawable.ic_menu_view), "Mark as read", pi);
        if (Build.VERSION.SDK_INT >= 28) {
            a.setSemanticAction(Notification.Action.SEMANTIC_ACTION_MARK_AS_READ);
        }
        return a.build();
    }

    /// Tapping the body opens the app on that conversation. The extra is read
    /// by the activity's intent; FLAG_IMMUTABLE because nothing adds to it.
    static PendingIntent openIntent(Context c, String roomId, int id) {
        Intent open = c.getPackageManager().getLaunchIntentForPackage(c.getPackageName());
        if (open == null) {
            open = new Intent(Intent.ACTION_MAIN).setPackage(c.getPackageName());
        }
        open.putExtra("room", roomId == null ? "" : roomId);
        open.setFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        return PendingIntent.getActivity(c, id, open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
    }

    // -------------------------------------------------------------- summary

    /// Android hides a group's summary while only one child is showing and
    /// draws it as soon as there are two, so it is posted whenever more than
    /// one room stands and taken away when they fall back to one.
    private static void summary(Context c) {
        NotificationManager nm = manager(c);
        if (nm == null) return;
        if (ROOMS.size() < 2) {
            nm.cancel(SUMMARY_TAG, SUMMARY_ID);
            return;
        }
        Notification.InboxStyle inbox = new Notification.InboxStyle();
        int n = 0;
        for (Map.Entry<String, Room> e : ROOMS.entrySet()) {
            Room r = e.getValue();
            if (r.msgs.isEmpty()) continue;
            inbox.addLine(r.name + ": " + r.msgs.get(r.msgs.size() - 1).text);
            n++;
        }
        Notification s = new Notification.Builder(c, CH_MESSAGES)
                .setSmallIcon(smallIcon(c))
                .setStyle(inbox)
                .setContentTitle(n + " conversations")
                .setGroup(GROUP)
                .setGroupSummary(true)
                .setAutoCancel(true)
                .setCategory(Notification.CATEGORY_MESSAGE)
                .setContentIntent(openIntent(c, "", SUMMARY_ID))
                .build();
        nm.notify(SUMMARY_TAG, SUMMARY_ID, s);
    }

    // -------------------------------------------------------------- cancel

    /// The room was opened, read, or replied to: take its notification away.
    public static synchronized void cancel(Context c, String roomId) {
        if (c == null || roomId == null) return;
        NotificationManager nm = manager(c);
        if (nm == null) return;
        try {
            nm.cancel(roomId, idOf(roomId));
            ROOMS.remove(roomId);
            summary(c);
        } catch (Throwable t) {
            Log.w(TAG, "cancel: " + t);
        }
    }

    /// Signed out, or notifications turned off: everything goes, the
    /// service's own notification excepted — the service cancels that itself
    /// when it stops.
    public static synchronized void cancelAll(Context c) {
        if (c == null) return;
        NotificationManager nm = manager(c);
        if (nm == null) return;
        try {
            for (String roomId : new ArrayList<String>(ROOMS.keySet())) {
                nm.cancel(roomId, idOf(roomId));
            }
            ROOMS.clear();
            nm.cancel(SUMMARY_TAG, SUMMARY_ID);
        } catch (Throwable t) {
            Log.w(TAG, "cancelAll: " + t);
        }
    }

    // ------------------------------------------------------------- service

    /// Start the foreground service that keeps the process — and with it the
    /// engine's socket — alive while the app is in the background.
    public static void startForeground(Context c) {
        if (c == null) return;
        try {
            ensureChannels(c);
            Intent i = new Intent(c, SigilService.class);
            if (Build.VERSION.SDK_INT >= 26) {
                c.startForegroundService(i);
            } else {
                c.startService(i);
            }
        } catch (Throwable t) {
            // API 31+ refuses to start a foreground service from the
            // background; the app is on screen when this is called, but a
            // refusal must never take the process with it.
            Log.w(TAG, "service start: " + t);
        }
    }

    public static void stopForeground(Context c) {
        if (c == null) return;
        try {
            c.stopService(new Intent(c, SigilService.class));
        } catch (Throwable t) {
            Log.w(TAG, "service stop: " + t);
        }
    }

    // ---------------------------------------------------------------- bits

    /// The notification id for a room. The TAG is the room id, which is what
    /// actually keeps two rooms apart; the id is derived from it so that the
    /// PendingIntents of two rooms differ by request code as well (a
    /// PendingIntent's extras are not part of its identity).
    static int idOf(String roomId) {
        int h = roomId.hashCode();
        if (h == SUMMARY_ID || h == SERVICE_ID) h = h + 7919;
        return h;
    }

    /// The app's own launcher icon. cargo-apk gives the manifest no icon of
    /// its own, so this is whatever the application entry resolved to; a
    /// notification with icon 0 is dropped by the framework, hence the
    /// fallback.
    static int smallIcon(Context c) {
        int icon = c.getApplicationInfo().icon;
        return icon != 0 ? icon : android.R.drawable.sym_def_app_icon;
    }

    static NotificationManager manager(Context c) {
        return (NotificationManager) c.getSystemService(Context.NOTIFICATION_SERVICE);
    }

    /// The sender's face as a round Icon, or null when there is no file.
    ///
    /// IconCompat is not on this class path, so the rounding is done here:
    /// createWithAdaptiveBitmap would mask and inset the bitmap AGAIN, which
    /// on an already-round avatar reads as a shrunken face in a grey square.
    /// A circle drawn by hand and createWithBitmap is what the shade wants.
    static synchronized Icon face(String path) {
        if (path == null || path.isEmpty()) return null;
        if (FACES.containsKey(path)) return FACES.get(path);
        Icon icon = null;
        try {
            File f = new File(path);
            if (f.isFile()) {
                BitmapFactory.Options probe = new BitmapFactory.Options();
                probe.inJustDecodeBounds = true;
                BitmapFactory.decodeFile(path, probe);
                int side = Math.max(probe.outWidth, probe.outHeight);
                BitmapFactory.Options o = new BitmapFactory.Options();
                o.inSampleSize = Math.max(1, side / 256);
                Bitmap src = BitmapFactory.decodeFile(path, o);
                if (src != null) {
                    Bitmap round = round(src, 192);
                    if (round != null) icon = Icon.createWithBitmap(round);
                    if (round != src) src.recycle();
                }
            }
        } catch (Throwable t) {
            Log.w(TAG, "face " + path + ": " + t);
        }
        // Remembered even when it failed: a missing file will not appear.
        if (FACES.size() > 64) FACES.clear();
        FACES.put(path, icon);
        return icon;
    }

    /// Centre-crop to a square and mask to a circle.
    private static Bitmap round(Bitmap src, int side) {
        int w = src.getWidth(), h = src.getHeight();
        if (w <= 0 || h <= 0) return null;
        int edge = Math.min(w, h);
        Rect from = new Rect((w - edge) / 2, (h - edge) / 2,
                (w - edge) / 2 + edge, (h - edge) / 2 + edge);
        Bitmap out = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888);
        Canvas canvas = new Canvas(out);
        Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        paint.setFilterBitmap(true);
        RectF to = new RectF(0, 0, side, side);
        canvas.drawOval(to, paint);
        paint.setXfermode(new PorterDuffXfermode(PorterDuff.Mode.SRC_IN));
        canvas.drawBitmap(src, from, to, paint);
        return out;
    }

    /// The long-lived shortcut a conversation notification hangs off.
    ///
    /// setShortcutId alone is inert: from API 30 the shade only files a
    /// notification under Conversations when a dynamic, long-lived shortcut
    /// of that id exists. It is pushed here rather than kept in sync
    /// elsewhere, because the only conversations that need one are the ones
    /// that have just spoken.
    private static void shortcut(Context c, String roomId, Room r, Msg last) {
        if (Build.VERSION.SDK_INT < 30) return;
        try {
            ShortcutManager sm = (ShortcutManager) c.getSystemService(ShortcutManager.class);
            if (sm == null) return;
            Intent open = new Intent(Intent.ACTION_VIEW)
                    .setPackage(c.getPackageName())
                    .putExtra("room", roomId);
            ShortcutInfo.Builder b = new ShortcutInfo.Builder(c, roomId)
                    .setShortLabel(r.name.isEmpty() ? "Conversation" : r.name)
                    .setLongLived(true)
                    .setIntent(open);
            Icon face = face(last.avatar);
            if (face != null) b.setIcon(face);
            Person.Builder p = new Person.Builder().setName(
                    last.sender.isEmpty() ? r.name : last.sender).setKey(roomId);
            if (face != null) p.setIcon(face);
            b.setPerson(p.build());
            sm.pushDynamicShortcut(b.build());
        } catch (Throwable t) {
            // A shortcut that could not be pushed costs the conversation
            // treatment, not the notification.
            Log.w(TAG, "shortcut: " + t);
        }
    }

    /// The text the shade typed into a reply, out of the broadcast the
    /// receiver was handed. Here rather than in SigilReceiver so that the
    /// RemoteInput key is named in exactly one place.
    static String replyText(Intent intent) {
        Bundle results = RemoteInput.getResultsFromIntent(intent);
        if (results == null) return "";
        CharSequence text = results.getCharSequence(KEY_TEXT);
        return text == null ? "" : text.toString().trim();
    }
}
