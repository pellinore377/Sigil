// The Storage Access Framework and camera round trips that
// android.app.NativeActivity cannot make on its own.
//
// Choosing or capturing media on Android is startActivityForResult, and the
// answer comes back as Activity.onActivityResult. Three things stand in the way:
//
//   * The activity is android.app.NativeActivity — a framework class named
//     directly in the manifest by cargo-apk (android/tools/cargo-apk, apk.rs),
//     so there is no subclass of ours to override onActivityResult in.
//   * android-activity 0.6 does not surface an activity result at all; its
//     lifecycle events stop at the ones ANativeActivityCallbacks defines, and
//     onActivityResult is not one of them.
//   * The Slint backend's own Java helper is not an activity either.
//
// The way through is android.app.Fragment. Activity.dispatchActivityResult
// routes a result whose request came from a fragment straight to that
// fragment's onActivityResult and never calls the activity's. So a throwaway
// fragment fires the intent and catches the answer, and nothing has to
// subclass NativeActivity.
//
// Four modes, all landing in the same place:
//
//   file   ACTION_OPEN_DOCUMENT over every type      — the Files tile
//   media  the photo picker, or OPEN_DOCUMENT filtered to
//          images and video, with multiple selection — the Gallery tile
//   photo  ACTION_IMAGE_CAPTURE                      — the Camera tile
//   video  ACTION_VIDEO_CAPTURE
//
// Everything after the answer happens here too, rather than back over JNI.
// Android hands back content:// URIs, and the engine's send path wants real
// files: core/src/sigil/mod.rs's attachment_send refuses a path that is not a
// file, and reads the name off it and the MIME type from its extension. So each
// document is read through the ContentResolver and copied into the app's own
// cache under its display name, extension and all. Doing that conversation over
// JNI would be forty calls per file; here it is one method.
//
// The capture modes need somewhere for the camera app to write. A FileProvider
// is the usual answer and is not available to us: androidx is not on the class
// path, and a provider named in the manifest must be loadable by the APK's own
// class loader, which cannot see the dex this class arrives in. So the output is
// a MediaStore row instead — a content:// URI the camera app can write to with
// no provider and no storage permission of ours, and which leaves the shot in
// the phone's gallery the way any camera app would.
//
// Rust starts a run with start() and then polls state() and takes paths() — the
// same shape as the location permission in core/src/geo/android.rs, which
// watches for the grant rather than receiving the callback it cannot register.

import android.app.Activity;
import android.app.Fragment;
import android.app.FragmentManager;
import android.content.ClipData;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;
import android.provider.OpenableColumns;
import android.util.Log;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;

public class SigilFilePicker extends Fragment {
    /** The tag logcat carries, so a pick reads next to the Rust side's lines. */
    private static final String TAG = "sigil";

    private static final String FRAGMENT_TAG = "sigil-file-picker";

    /** Ours alone; the fragment only ever sees results it asked for. */
    private static final int REQUEST = 7003;

    // What Rust polls. Anything other than WAITING ends the wait.
    public static final int IDLE = 0;
    public static final int WAITING = 1;
    public static final int READY = 2;
    public static final int NOTHING = 3;

    // The modes Rust asks for, by name rather than by number so a log line reads.
    private static final String MODE_FILE = "file";
    private static final String MODE_MEDIA = "media";
    private static final String MODE_PHOTO = "photo";
    private static final String MODE_VIDEO = "video";

    /**
     * Rust joins the answer on this, and the copy strips it out of any name, so
     * a file whose name contained one could not split a path in two. It is the
     * separator the app packs lists with elsewhere (create_poll in actions.rs).
     */
    private static final char SEP = (char) 0x1f;

    /** What the photo picker will accept in one go. */
    private static final int MAX_ITEMS = 10;

    private static volatile int sState = IDLE;
    private static volatile String sPaths = "";

    /** Where the chosen bytes land. Rust owns the directory and passes it in. */
    private static volatile String sDir = "";

    private static volatile String sMode = MODE_FILE;

    /** For the capture modes: the MediaStore row the camera app writes into. */
    private static volatile Uri sCaptureTarget = null;

    /** WAITING while a run is open, then READY or NOTHING. */
    public static synchronized int state() {
        return sState;
    }

    /**
     * The copied files' absolute paths, joined on the unit separator, once
     * state() has left WAITING; empty if the person backed out or every copy
     * failed. Reading it clears the slot, so a stale answer can never be handed
     * to the next run.
     */
    public static synchronized String paths() {
        String p = sPaths;
        sPaths = "";
        sState = IDLE;
        return p;
    }

    /**
     * Open the picker or the camera. Safe to call from any thread: the fragment
     * transaction is posted to the UI thread, which is the only one
     * FragmentManager accepts.
     */
    public static synchronized void start(final Activity activity, String dir, String mode) {
        if (sState == WAITING) {
            Log.w(TAG, "file picker: a pick is already open");
            return;
        }
        sDir = dir;
        sMode = (mode == null || mode.isEmpty()) ? MODE_FILE : mode;
        sPaths = "";
        sCaptureTarget = null;
        sState = WAITING;
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                attach(activity);
            }
        });
    }

    /** Put the fragment on the activity, then let it fire the intent. UI thread. */
    private static void attach(Activity activity) {
        try {
            FragmentManager fm = activity.getFragmentManager();
            // A run that was interrupted can leave one behind; never stack two.
            Fragment old = fm.findFragmentByTag(FRAGMENT_TAG);
            if (old != null) {
                fm.beginTransaction().remove(old).commitAllowingStateLoss();
                fm.executePendingTransactions();
            }
            SigilFilePicker f = new SigilFilePicker();
            fm.beginTransaction().add(f, FRAGMENT_TAG).commitAllowingStateLoss();
            // startActivityForResult needs a host, which the fragment only has
            // once the transaction has actually run.
            fm.executePendingTransactions();
            f.launch();
        } catch (Throwable t) {
            Log.e(TAG, "file picker: could not attach the fragment", t);
            finish(NOTHING, "");
        }
    }

    private void launch() {
        try {
            Intent intent = intentFor(getActivity(), sMode);
            if (intent == null) {
                finish(NOTHING, "");
                return;
            }
            startActivityForResult(intent, REQUEST);
            Log.i(TAG, "file picker: " + sMode + " is open");
        } catch (Throwable t) {
            Log.e(TAG, "file picker: nothing on this phone handles " + sMode, t);
            releaseTarget(getActivity());
            detach();
            finish(NOTHING, "");
        }
    }

    private static Intent intentFor(Activity activity, String mode) {
        if (MODE_PHOTO.equals(mode) || MODE_VIDEO.equals(mode)) {
            return captureIntent(activity, MODE_VIDEO.equals(mode));
        }
        if (MODE_MEDIA.equals(mode)) {
            return galleryIntent();
        }
        // Files: everything, and CATEGORY_OPENABLE so the provider can give us a
        // stream to copy. Needs no storage permission.
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        return intent;
    }

    /**
     * Pictures and video, several at a time.
     *
     * The system photo picker is the better UI and the one the design is drawn
     * from, but it only exists from API 33; below that — which includes the
     * phone this is tested on — the document picker filtered to image and video
     * is the same choice through a plainer front end.
     */
    private static Intent galleryIntent() {
        if (Build.VERSION.SDK_INT >= 33) {
            Intent intent = new Intent(MediaStore.ACTION_PICK_IMAGES);
            intent.putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, MAX_ITEMS);
            return intent;
        }
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        intent.putExtra(Intent.EXTRA_MIME_TYPES, new String[] { "image/*", "video/*" });
        intent.putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true);
        return intent;
    }

    /**
     * The system camera, writing into a MediaStore row of ours.
     *
     * Deliberately no CAMERA permission: handing the job to another app needs
     * none, and declaring it in the manifest would turn it into a runtime grant
     * the person has to answer before the camera would open at all.
     */
    private static Intent captureIntent(Activity activity, boolean video) {
        Uri target = newCaptureTarget(activity, video);
        if (target == null) {
            Log.e(TAG, "file picker: MediaStore would not make a row for the camera");
            return null;
        }
        sCaptureTarget = target;
        Intent intent = new Intent(video ? MediaStore.ACTION_VIDEO_CAPTURE : MediaStore.ACTION_IMAGE_CAPTURE);
        intent.putExtra(MediaStore.EXTRA_OUTPUT, target);
        // The grant flags only reach a URI the intent actually carries, and
        // EXTRA_OUTPUT is just an extra — the clip data is what makes the
        // camera app's write permission stick.
        intent.setClipData(ClipData.newRawUri("", target));
        intent.addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION | Intent.FLAG_GRANT_READ_URI_PERMISSION);
        return intent;
    }

    /**
     * A row in the phone's gallery for the camera app to fill.
     *
     * From API 29 this needs no permission of ours. Below it, inserting wants
     * WRITE_EXTERNAL_STORAGE, which the app does not ask for — so on API 26 to
     * 28 capture reports nothing rather than crashing, and the gallery and files
     * routes still work.
     */
    private static Uri newCaptureTarget(Activity activity, boolean video) {
        try {
            String stamp = "SIGIL_" + System.currentTimeMillis();
            ContentValues values = new ContentValues();
            values.put(MediaStore.MediaColumns.DISPLAY_NAME, stamp + (video ? ".mp4" : ".jpg"));
            values.put(MediaStore.MediaColumns.MIME_TYPE, video ? "video/mp4" : "image/jpeg");
            if (Build.VERSION.SDK_INT >= 29) {
                String folder = video ? Environment.DIRECTORY_MOVIES : Environment.DIRECTORY_PICTURES;
                values.put(MediaStore.MediaColumns.RELATIVE_PATH, folder + "/Sigil");
            }
            Uri collection = video
                    ? MediaStore.Video.Media.EXTERNAL_CONTENT_URI
                    : MediaStore.Images.Media.EXTERNAL_CONTENT_URI;
            return activity.getContentResolver().insert(collection, values);
        } catch (Throwable t) {
            Log.e(TAG, "file picker: could not reserve a MediaStore row"
                    + (Build.VERSION.SDK_INT < 29 ? " (Android 9 or older needs the storage permission)" : ""), t);
            return null;
        }
    }

    /** Drop the reserved row when the camera comes back with nothing in it. */
    private static void releaseTarget(Activity activity) {
        Uri target = sCaptureTarget;
        sCaptureTarget = null;
        if (target == null || activity == null) {
            return;
        }
        try {
            activity.getContentResolver().delete(target, null, null);
        } catch (Throwable t) {
            Log.w(TAG, "file picker: could not drop the unused MediaStore row", t);
        }
    }

    @Override
    public void onActivityResult(int requestCode, int resultCode, Intent data) {
        if (requestCode != REQUEST) {
            super.onActivityResult(requestCode, resultCode, data);
            return;
        }
        final Activity activity = getActivity();
        final boolean ok = resultCode == Activity.RESULT_OK;
        final List<Uri> uris = new ArrayList<Uri>();
        if (ok) {
            // A capture writes into the row we reserved and returns no data of
            // its own; a pick returns one URI, or several in the clip data.
            Uri target = sCaptureTarget;
            if (target != null) {
                uris.add(target);
                sCaptureTarget = null;
            } else if (data != null) {
                ClipData clip = data.getClipData();
                if (clip != null) {
                    for (int i = 0; i < clip.getItemCount(); i++) {
                        Uri u = clip.getItemAt(i).getUri();
                        if (u != null) {
                            uris.add(u);
                        }
                    }
                } else if (data.getData() != null) {
                    uris.add(data.getData());
                }
            }
        }
        if (!ok) {
            releaseTarget(activity);
        }
        // Off the activity before anything slow: see detach().
        detach();

        if (uris.isEmpty() || activity == null) {
            Log.i(TAG, "file picker: nothing was chosen");
            finish(NOTHING, "");
            return;
        }
        // The documents may be megabytes each, and on a cloud provider every
        // read is a network round trip. Never on the UI thread.
        new Thread(new Runnable() {
            @Override
            public void run() {
                StringBuilder out = new StringBuilder();
                for (Uri uri : uris) {
                    try {
                        String path = copy(activity, uri);
                        if (!path.isEmpty()) {
                            if (out.length() > 0) {
                                out.append(SEP);
                            }
                            out.append(path);
                        }
                    } catch (Throwable t) {
                        Log.e(TAG, "file picker: could not read " + uri, t);
                    }
                }
                String joined = out.toString();
                finish(joined.isEmpty() ? NOTHING : READY, joined);
            }
        }, "sigil-file-copy").start();
    }

    /**
     * Take the fragment off the activity as soon as it has done its job.
     *
     * This matters more than it looks: a fragment still attached when the
     * activity saves its state is written into that state by class name, and
     * this class lives in a dex loaded at runtime (src/platform.rs) that the
     * activity's own class loader cannot see. Were the process killed and the
     * activity rebuilt from that state, the framework could not find the class.
     * Detaching the moment the answer arrives keeps the window to the length of
     * the pick itself.
     */
    private void detach() {
        try {
            FragmentManager fm = getFragmentManager();
            if (fm != null) {
                fm.beginTransaction().remove(this).commitAllowingStateLoss();
            }
        } catch (Throwable t) {
            Log.w(TAG, "file picker: could not detach the fragment", t);
        }
    }

    /**
     * The document's bytes as a real file, under its own display name.
     *
     * The name has to survive the copy: the engine takes the attachment's
     * filename from it and guesses the MIME type from its extension. Each file
     * gets its own directory so two called the same thing cannot collide.
     */
    private static String copy(Activity activity, Uri uri) throws IOException {
        ContentResolver resolver = activity.getContentResolver();

        String name = displayName(resolver, uri);
        // Nothing from a provider is trusted as a path: a separator would write
        // outside the directory we chose, and a control character would break
        // the list Rust unpacks.
        StringBuilder clean = new StringBuilder();
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            clean.append(c < 0x20 || c == '/' || c == '\\' || c == 0x7f ? '_' : c);
        }
        name = clean.toString().trim();
        if (name.isEmpty() || name.equals(".") || name.equals("..")) {
            name = "attachment";
        }
        if (name.lastIndexOf('.') <= 0) {
            // No extension means the engine would guess application/octet-stream
            // and a picture would arrive as a file. Ask the type instead.
            String ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(resolver.getType(uri));
            if (ext != null && !ext.isEmpty()) {
                name = name + "." + ext;
            }
        }

        File dir = new File(sDir, Long.toString(System.currentTimeMillis()) + "-" + Math.abs(uri.hashCode()));
        if (!dir.mkdirs() && !dir.isDirectory()) {
            throw new IOException("cannot create " + dir);
        }
        File out = new File(dir, name);

        InputStream in = resolver.openInputStream(uri);
        if (in == null) {
            throw new IOException("the provider gave no stream for " + uri);
        }
        try {
            OutputStream os = new FileOutputStream(out);
            try {
                byte[] buf = new byte[64 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) {
                    os.write(buf, 0, n);
                }
                os.flush();
            } finally {
                os.close();
            }
        } finally {
            in.close();
        }

        if (out.length() == 0) {
            // A camera that was cancelled mid-shot leaves the row empty; sending
            // a zero-byte attachment would be worse than sending nothing.
            Log.w(TAG, "file picker: " + uri + " was empty");
            out.delete();
            return "";
        }
        Log.i(TAG, "file picker: copied " + out.length() + " bytes to " + out.getAbsolutePath());
        return out.getAbsolutePath();
    }

    /** OpenableColumns.DISPLAY_NAME, falling back to the URI's last segment. */
    private static String displayName(ContentResolver resolver, Uri uri) {
        Cursor c = null;
        try {
            c = resolver.query(uri, new String[] { OpenableColumns.DISPLAY_NAME }, null, null, null);
            if (c != null && c.moveToFirst()) {
                int i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME);
                if (i >= 0) {
                    String n = c.getString(i);
                    if (n != null && !n.trim().isEmpty()) {
                        return n.trim();
                    }
                }
            }
        } catch (Throwable t) {
            Log.w(TAG, "file picker: the provider has no display name", t);
        } finally {
            if (c != null) {
                c.close();
            }
        }
        String last = uri.getLastPathSegment();
        if (last == null) {
            return "";
        }
        int slash = last.lastIndexOf('/');
        return slash >= 0 ? last.substring(slash + 1) : last;
    }

    private static synchronized void finish(int state, String paths) {
        sPaths = paths;
        sState = state;
    }
}
