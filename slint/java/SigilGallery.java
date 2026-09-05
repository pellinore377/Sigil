// Saving a picture or a clip where the phone's own gallery will find it.
//
// The viewer's download used to fire an engine request that does not exist
// (`media.saveAs`), and its destination was $HOME/Downloads — which on Android
// is the app's private files directory, a place no gallery ever looks. Both
// halves were wrong: there is no such request, and there is no such place.
//
// What a phone actually wants is a MediaStore row. From API 29 an app may
// insert into the shared collections WITHOUT any storage permission, so long
// as it only writes rows of its own: put the row in first with IS_PENDING set,
// stream the bytes into the URI the insert answered with, then clear IS_PENDING
// and the picture appears in Photos. RELATIVE_PATH decides which album it
// lands in — Pictures/Sigil for a still, Movies/Sigil for a clip.
//
// Below API 29 there is no RELATIVE_PATH and no IS_PENDING: the file has to be
// written into the public directory by hand and the row made to point at it,
// which DOES need WRITE_EXTERNAL_STORAGE. This app does not ask for that
// permission, so that path is attempted and its failure reported rather than
// pretended away — every device this runs on is far past 29.
//
// Loaded at runtime from the embedded dex (build.rs, platform.rs) beside
// SigilFilePicker, SigilVideo and SigilCamera. Everything is static and called
// from the engine's threads; nothing here touches a view, so none of it needs
// the main thread.

import android.app.Activity;
import android.content.ContentResolver;
import android.content.ContentValues;
import android.net.Uri;
import android.os.Build;
import android.os.Environment;
import android.provider.MediaStore;
import android.webkit.MimeTypeMap;

import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;

public final class SigilGallery {
    private SigilGallery() {}

    /// Put `path` in the gallery. Answers "" when it is there, and why not
    /// otherwise — the caller shows that to the person, so it is a sentence
    /// rather than an exception.
    public static String save(Activity activity, String path, String mime, String name) {
        File src = new File(path == null ? "" : path);
        if (!src.isFile()) return "that file is not here any more";

        String type = pick(mime, src.getName());
        String display = safeName(name, src.getName(), type);

        try {
            if (Build.VERSION.SDK_INT >= 29) {
                return modern(activity, src, type, display);
            }
            return legacy(activity, src, type, display);
        } catch (SecurityException e) {
            return "Sigil is not allowed to write to the gallery";
        } catch (Exception e) {
            return "that could not be saved: " + e;
        }
    }

    /// API 29 and up: a pending row, the bytes, then the row published. No
    /// storage permission is involved — an app may always insert its own.
    private static String modern(Activity a, File src, String type,
                                 String display) throws Exception {
        ContentResolver r = a.getContentResolver();

        ContentValues v = new ContentValues();
        v.put(MediaStore.MediaColumns.DISPLAY_NAME, display);
        v.put(MediaStore.MediaColumns.MIME_TYPE, type);
        v.put(MediaStore.MediaColumns.RELATIVE_PATH, album(type));
        // Hidden from the gallery until every byte is in: a half-copied
        // picture in Photos is worse than none.
        v.put(MediaStore.MediaColumns.IS_PENDING, 1);

        Uri row = r.insert(collection(type), v);
        if (row == null) return "the gallery would not take it";
        try {
            OutputStream out = r.openOutputStream(row);
            if (out == null) throw new java.io.IOException("no stream for " + row);
            copy(src, out);
            ContentValues done = new ContentValues();
            done.put(MediaStore.MediaColumns.IS_PENDING, 0);
            r.update(row, done, null, null);
            return "";
        } catch (Exception e) {
            // A row nobody can see is litter; take it back out.
            try { r.delete(row, null, null); } catch (RuntimeException ignored) {}
            throw e;
        }
    }

    /// Before API 29: write the file into the public album and tell MediaStore
    /// where it is. Needs WRITE_EXTERNAL_STORAGE, which this app does not ask
    /// for, so this is here to fail honestly rather than to work.
    @SuppressWarnings("deprecation")
    private static String legacy(Activity a, File src, String type,
                                 String display) throws Exception {
        File dir = new File(Environment.getExternalStoragePublicDirectory(
                publicDir(type)), "Sigil");
        if (!dir.mkdirs() && !dir.isDirectory()) {
            return "the gallery folder could not be made";
        }
        File out = new File(dir, display);
        OutputStream os = new FileOutputStream(out);
        copy(src, os);

        ContentValues v = new ContentValues();
        v.put(MediaStore.MediaColumns.DISPLAY_NAME, display);
        v.put(MediaStore.MediaColumns.MIME_TYPE, type);
        v.put(MediaStore.MediaColumns.DATA, out.getAbsolutePath());
        if (a.getContentResolver().insert(collection(type), v) == null) {
            return "the gallery would not take it";
        }
        return "";
    }

    /// Which shared collection a type belongs in. A picture and a clip go to
    /// the gallery's own; everything else — a document, a voice note — goes to
    /// Downloads, which is where a phone expects to find such a thing and
    /// which the gallery correctly ignores.
    private static Uri collection(String type) {
        if (type.startsWith("image/")) return MediaStore.Images.Media.EXTERNAL_CONTENT_URI;
        if (type.startsWith("video/")) return MediaStore.Video.Media.EXTERNAL_CONTENT_URI;
        if (Build.VERSION.SDK_INT >= 29) return MediaStore.Downloads.EXTERNAL_CONTENT_URI;
        return MediaStore.Files.getContentUri("external");
    }

    private static String album(String type) {
        return publicDir(type) + File.separator + "Sigil";
    }

    private static String publicDir(String type) {
        if (type.startsWith("image/")) return Environment.DIRECTORY_PICTURES;
        if (type.startsWith("video/")) return Environment.DIRECTORY_MOVIES;
        return Environment.DIRECTORY_DOWNLOADS;
    }

    private static void copy(File src, OutputStream out) throws Exception {
        InputStream in = new FileInputStream(src);
        try {
            byte[] buf = new byte[64 * 1024];
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            out.flush();
        } finally {
            try { in.close(); } catch (Exception ignored) {}
            try { out.close(); } catch (Exception ignored) {}
        }
    }

    /// The type the caller gave, or the one the extension implies, or a still
    /// — MediaStore refuses a row whose MIME does not match its collection.
    private static String pick(String mime, String fallbackName) {
        if (mime != null && mime.contains("/")) return mime;
        String ext = extension(fallbackName);
        String guess = ext.isEmpty() ? null
                : MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext);
        return guess == null || !guess.contains("/") ? "image/jpeg" : guess;
    }

    private static String extension(String name) {
        int dot = name == null ? -1 : name.lastIndexOf('.');
        if (dot <= 0 || dot == name.length() - 1) return "";
        return name.substring(dot + 1).toLowerCase(java.util.Locale.ROOT);
    }

    /// A filename that cannot escape the album it is going into, with an
    /// extension the type agrees with. Nothing from a message is trusted as a
    /// path: a separator would write somewhere else entirely.
    private static String safeName(String name, String fallback, String type) {
        String n = name == null || name.trim().isEmpty() ? fallback : name;
        StringBuilder clean = new StringBuilder();
        for (int i = 0; i < n.length(); i++) {
            char c = n.charAt(i);
            clean.append(c < 0x20 || c == '/' || c == '\\' || c == 0x7f ? '_' : c);
        }
        n = clean.toString().trim();
        if (n.isEmpty() || n.equals(".") || n.equals("..")) n = "sigil";
        if (extension(n).isEmpty()) {
            String ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(type);
            n = n + "." + (ext == null || ext.isEmpty() ? "jpg" : ext);
        }
        return n;
    }
}
