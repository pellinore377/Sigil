// The camera, seen by the phone: the reference shot, built as views.
//
// WHY THIS IS A WINDOW OF ITS OWN. The app is a NativeActivity: it calls
// Window.takeSurface() and paints the activity window's surface itself,
// through EGL, from Rust. Two things follow, and between them they decide the
// whole shape of this file.
//
//   * Ordinary Android views added to the activity (addContentView) are laid
//     out but never drawn — ViewRootImpl does not own that surface any more.
//     So a TextView over the activity would be invisible.
//   * A SurfaceView gets a surface of its own, so it IS drawn; that is how the
//     old letterboxed preview worked. But its z-order is a SUBLAYER of the
//     activity window: setZOrderMediaOverlay(true) is sublayer −1, BELOW the
//     window the app paints, so the camera would vanish behind the app; only
//     setZOrderOnTop(true) (sublayer +1) is above it, and then nothing the app
//     draws can ever be on top of the picture. That is exactly the trap the
//     first version fell into, and why the controls ended up under the box.
//
// The way out is a second WINDOW. This overlay is added straight to the
// WindowManager as TYPE_APPLICATION_SUB_PANEL — sublayer +2, above the
// activity window AND above any of its SurfaceViews — with the activity's own
// token, so it lives and dies with the activity. That window is an ordinary
// one: its ViewRootImpl draws it, so ordinary views work again. Inside it the
// preview is a plain SurfaceView at the DEFAULT z (sublayer −2 of THIS
// window), which punches a transparent hole through the window's own surface;
// every sibling added after it paints over that hole.
//
// THE COMPOSITION IS THE REFERENCE'S, measured off it (see the `dp` block
// below). The one thing worth saying here, because everything else hangs off
// it: the picture is not "most of the screen" — it is the 4:3 preview AT FULL
// WIDTH. 1344 × 4/3 = 1792, and 1792 is exactly where the reference's picture
// ends. So the picture box is W × W·4/3 on any phone, the controls hang off
// its FOOT (mode 32, shutter 112, zoom 195 above it), and what is left below
// is the gallery sheet.
//
//     overlay window (SUB_PANEL, translucent, no limits, over the cutout)
//     └── FrameLayout ................ black ground; eats every touch
//         ├── pictureBox ............. W × W·4/3, clips its child
//         │   └── SurfaceView ........ the preview, fitted (a 4:3 preview in
//         │                            a 3:4 box fills it exactly)
//         ├── FrameView .............. the picture's 28dp rounded bottom edge
//         ├── sheet .................. the gallery, 16dp under the foot
//         │   ├── the drag handle .... 32 × 4dp pill
//         │   └── ScrollView ......... 3 columns of square thumbnails
//         ├── TextView ............... "Starting the camera…" / the failure
//         ├── recording pill ......... red dot + elapsed, top centre
//         ├── IconView(CLOSE) ........ top left
//         ├── IconView(FLASH) ........ top right
//         ├── LinearLayout ........... the zoom pill: 0.5 / 1.0 / 2.0
//         ├── ShutterView ............ the big white ring
//         ├── IconView(FLIP) ......... the flip ring beside it
//         └── LinearLayout ........... Photo | Video
//
// FLAG_LAYOUT_NO_LIMITS and LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS put the
// window under the status bar and the gesture bar, as the reference does. The
// picture runs up under the status bar; the close and the flash are inset off
// the real top inset so neither sits under it, and the gallery's last row
// scrolls clear of the gesture bar on the bottom one.
//
// WHAT THIS FILE DECIDES AND WHAT IT DOES NOT. Every control here is wired
// straight to the session — zoom, torch, flip, mode — because they are the
// camera's own business. Three things are not, and they are the whole seam to
// Rust: the SHUTTER bumps a counter (shutterCount), the X raises a flag
// (closed), and a tapped thumbnail is copied to a file whose name is published
// (pickedPath). The bridge polls all three and answers, so a shot and a pick
// both land on the staging page by the route every other attachment takes.
//
// The session shape is unchanged. One CameraDevice and one preview surface,
// with the capture session configured two ways and swapped between them:
//
//   still  — [preview, ImageReader(JPEG)]   TEMPLATE_PREVIEW repeating,
//                                           TEMPLATE_STILL_CAPTURE for a shot
//   record — [preview, MediaRecorder]       TEMPLATE_RECORD repeating
//
// They are two sessions rather than one three-output session because a
// MediaRecorder surface only exists between prepare() and stop(): it has to
// be built for each clip, with that clip's file and orientation on it, and a
// session cannot have an output swapped under it.
//
// Everything here is static and called from the engine's threads: every touch
// of a view hops to the main thread, every camera call hops to a HandlerThread
// of ours, every MediaStore read hops to a second one, and every question the
// bridge asks reads a volatile field. Loaded at runtime from the embedded dex
// (build.rs, platform.rs) alongside SigilFilePicker and SigilVideo, so nothing
// here is named in the manifest — the CAMERA and READ_MEDIA_* permissions
// are, and platform.rs asks for them before calling open().
//
// EVERY MEASUREMENT BELOW WAS TAKEN OFF THE REFERENCE SHOT with ImageMagick,
// at 1344×2992 and density 408 (1dp = 2.55px); the px figure is in the comment
// beside each dp.

import android.app.Activity;
import android.app.Application;
import android.content.ContentResolver;
import android.content.ContentUris;
import android.content.Context;
import android.database.Cursor;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.ImageFormat;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.drawable.GradientDrawable;
import android.hardware.camera2.CameraAccessException;
import android.hardware.camera2.CameraCaptureSession;
import android.hardware.camera2.CameraCharacteristics;
import android.hardware.camera2.CameraDevice;
import android.hardware.camera2.CameraManager;
import android.hardware.camera2.CameraMetadata;
import android.hardware.camera2.CaptureRequest;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.Image;
import android.media.ImageReader;
import android.media.MediaRecorder;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.Looper;
import android.provider.MediaStore;
import android.util.Range;
import android.util.Size;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.webkit.MimeTypeMap;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.List;

public final class SigilCamera {
    /// What state() answers. The bridge shows the first three and treats
    /// "error" as "close and toast".
    public static final String IDLE = "idle";
    public static final String OPENING = "opening";
    public static final String READY = "ready";
    public static final String CAPTURING = "capturing";
    public static final String RECORDING = "recording";
    public static final String ERROR = "error";

    // ------------------------------------------------------- the reference
    //
    // Measured off Screenshot_20260904-000346.png. Everything is stated in dp
    // and turned into pixels by dp() below, so the same numbers hold on any
    // density — except the picture's own height, which is a RATIO (see
    // pictureH) because that is what the reference actually is.

    /// The picture is the 4:3 preview at full width, so the box is 3:4.
    /// 1344 × 4/3 = 1792, and the reference's picture ends at exactly 1792.
    private static final float PICTURE_W_OVER_H = 3f / 4f;
    /// The shutter: outer ring Ø 194px, 8px stroke, a 17-18px gap, a 143px
    /// white disc.
    private static final float SHUTTER_D = 76f;      // 194px
    private static final float SHUTTER_STROKE = 3.1f;// 8px
    private static final float SHUTTER_INNER = 56f;  // 143px
    /// The flip ring: Ø 133px, 6px stroke, its glyph half the ring across.
    private static final float FLIP_D = 52f;         // 133px
    private static final float FLIP_STROKE = 2.35f;  // 6px
    private static final float FLIP_GLYPH = 26f;     // 66px
    /// The zoom pill: 341×79px, chips 48dp centre to centre, the lit one a
    /// 82px disc.
    private static final float PILL_H = 32f;         // 79px
    private static final float CHIP_D = 32f;         // 82px
    private static final float CHIP_GAP = 16f;       // centres 122.75px apart
    /// 2·6 + 3·82 + 2·41 = 340, against the 341 measured, and it puts the
    /// three chip centres on 549 / 672 / 795 (measured 549.5 / 671.5 / 795).
    private static final float PILL_PAD = 2.5f;      // 6px
    /// Photo | Video: the lit label in a 180×67px pill, the other 200px to its
    /// right, and the LIT one centred on the screen — which is what the
    /// reference shows, not a centred row.
    private static final float MODE_H = 26f;         // 67px
    private static final float MODE_PAD = 17f;       // (180−94)/2 px
    private static final float MODE_GAP = 27f;       // 68px between pill and label
    /// The close and the flash: a 35px glyph and a 49px one, both centred
    /// 73px in from their edge and 281px down.
    private static final float ICON_BOX = 48f;
    private static final float ICON_EDGE = 4f;       // → centre 28dp in (73px)
    private static final float ICON_DROP = 34f;      // below the status inset
    private static final float CLOSE_GLYPH = 14f;    // 35px
    private static final float FLASH_GLYPH = 20f;    // 49×52px with the slash
    private static final float STROKE = 2.4f;        // 6px, every drawn line
    /// Heights above the PICTURE'S FOOT: mode centre 82px, shutter centre
    /// 285px, zoom centre 498px.
    private static final float ROW_MODE = 32f;
    private static final float ROW_SHUTTER = 112f;
    private static final float ROW_ZOOM = 195f;
    /// The flip ring's centre, 384px right of the shutter's.
    private static final float FLIP_OFFSET = 151f;
    /// The picture's rounded bottom edge, and the sheet's rounded top: both a
    /// 72-73px radius.
    private static final float CORNER = 28f;

    /// The gallery sheet. Foot 1792 → the window's own black to 1832 → the
    /// sheet from 1833, its handle centred at 1888.5, its grid from 1951.
    private static final float SHEET_GAP = 16f;      // 41px
    private static final float HANDLE_W = 32f;       // 82px
    private static final float HANDLE_H = 4f;        // 10px
    private static final float HANDLE_TOP = 19.8f;   // pill top; centre 21.8
    private static final float SHEET_HEAD = 46f;     // 118px, sheet top → grid
    /// The grid: three square cells edge to edge with a 3px gutter between
    /// them and none at the sides. Cells measured 446 × 446px.
    private static final float GUTTER = 1.2f;        // 3px
    private static final int COLUMNS = 3;
    /// Enough to fill three screens of scrolling; more is a slideshow.
    private static final int GALLERY_MAX = 30;

    /// Colours, sampled off the reference. The lit chip and the lit mode pill
    /// are the SAME flat #9F9E97 over two very different scenes, so they are
    /// opaque, not a white at alpha; the zoom pill darkens whatever is under
    /// it, so it is a black at about 45%.
    private static final int ON_BG = 0xFF9F9E97;
    private static final int ON_FG = 0xFF20211C;
    private static final int OFF_FG = 0xFFE5E5E5;
    private static final int SCRIM = 0x73000000;
    private static final int REC = 0xFFE0403A;
    /// The window's own ground, the sheet's, and the handle's.
    private static final int GROUND = 0xFF0E0E0D;
    private static final int SHEET_BG = 0xFF20201D;
    private static final int HANDLE_C = 0xFF585753;

    private static final float[] STOPS = { 0.5f, 1.0f, 2.0f };

    // ----------------------------------------------------------- the state

    private static Activity host;
    private static Handler ui;
    private static HandlerThread thread;
    private static Handler bg;
    /// A second thread for MediaStore: a cursor and a bitmap decode must not
    /// sit in front of a camera callback.
    private static HandlerThread ioThread;
    private static Handler io;

    private static ViewGroup overlay;
    private static FrameLayout pictureBox;
    private static SurfaceView view;
    private static SurfaceHolder holder;
    private static FrameView frameView;
    private static TextView hint;
    private static View recPill;
    private static TextView recText;
    private static IconView closeBtn;
    private static IconView flashBtn;
    private static IconView flipBtn;
    private static ShutterView shutterBtn;
    private static LinearLayout zoomPill;
    private static TextView[] chips;
    private static LinearLayout modeRow;
    private static TextView[] modeLabels;
    private static ScrollView galleryScroll;
    private static LinearLayout galleryRows;
    private static Application.ActivityLifecycleCallbacks lifecycle;

    private static CameraDevice device;
    private static CameraCaptureSession session;
    private static ImageReader jpeg;
    private static MediaRecorder recorder;

    private static String cameraId;
    private static int sensorOrientation;
    private static boolean flashAvailable;
    private static boolean zoomRatioSupported;
    private static Rect activeArray;
    private static Size previewSize;
    private static Size jpegSize;
    private static Size videoSize;

    private static float density = 2.55f;
    /// The window, in physical pixels, the system-bar insets inside it, and
    /// the picture's foot — the line everything else is measured from.
    private static int winW, winH, insetTop, insetBottom, foot;
    /// Where a tapped thumbnail is copied to; handed down by the bridge, which
    /// is the only side that knows the engine's cache directory.
    private static String pickDir = "";

    /// The surface exists and has been given its buffer size: the camera may
    /// open. Set on the main thread, read on ours.
    private static volatile boolean surfaceReady;
    /// One open per surface: surfaceChanged fires again on every resize.
    private static volatile boolean opening;

    private static volatile boolean front;
    private static volatile float zoom = 1f;
    private static volatile float zoomMin = 1f;
    private static volatile float zoomMax = 1f;
    private static volatile boolean torch;
    private static volatile String mode = "photo";

    private static volatile String state = IDLE;
    private static volatile String lastPath = "";
    private static volatile String failure;
    /// Where the shot or the clip in flight is going.
    private static volatile String pendingPhoto = "";
    private static volatile String pendingVideo = "";

    /// The three things the overlay tells the bridge rather than doing itself.
    private static volatile int shutterCount;
    private static volatile boolean closed;
    private static volatile String pickedPath = "";

    private static long recordStart;

    private SigilCamera() {}

    // ------------------------------------------------------------- opening

    /// Put the viewfinder up over everything. `facing` is "front" or anything
    /// else for the back camera; `startMode` is "video" or anything else for
    /// stills; `dir` is where a tapped gallery item is copied to. A second
    /// call replaces the first.
    public static void open(final Activity activity, final String facing,
                            final String startMode, final String dir) {
        close();
        host = activity;
        front = "front".equals(facing);
        mode = "video".equals(startMode) ? "video" : "photo";
        pickDir = dir == null ? "" : dir;
        state = OPENING;
        failure = null;
        lastPath = "";
        pendingPhoto = "";
        pendingVideo = "";
        surfaceReady = false;
        opening = false;
        closed = false;
        pickedPath = "";
        // The bridge's baseline is zero: a press left over from the session
        // before must not read as a fresh one on the first pass.
        shutterCount = 0;
        zoom = 1f;
        torch = false;

        density = activity.getResources().getDisplayMetrics().density;
        measureWindow(activity);
        if (!pickCamera(activity)) return;

        thread = new HandlerThread("sigil-camera");
        thread.start();
        bg = new Handler(thread.getLooper());
        ioThread = new HandlerThread("sigil-camera-io");
        ioThread.start();
        io = new Handler(ioThread.getLooper());
        ui = new Handler(Looper.getMainLooper());

        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                build(activity);
            }
        });
    }

    /// The window's size and its system-bar insets, in physical pixels, and
    /// the picture's foot off them. Read off the ACTIVITY's decor rather than
    /// our own window, which has not been added yet — and which, being
    /// FLAG_LAYOUT_NO_LIMITS, would answer for a frame that ignores the bars
    /// anyway.
    private static void measureWindow(Activity a) {
        insetTop = 0;
        insetBottom = 0;
        try {
            View decor = a.getWindow().getDecorView();
            winW = decor.getWidth();
            winH = decor.getHeight();
            WindowInsets wi = decor.getRootWindowInsets();
            if (wi != null) {
                if (Build.VERSION.SDK_INT >= 30) {
                    android.graphics.Insets in =
                            wi.getInsets(WindowInsets.Type.systemBars());
                    insetTop = in.top;
                    insetBottom = in.bottom;
                } else {
                    insetTop = wi.getSystemWindowInsetTop();
                    insetBottom = wi.getSystemWindowInsetBottom();
                }
            }
        } catch (RuntimeException ignored) {
        }
        if (winW <= 0 || winH <= 0) {
            winW = a.getResources().getDisplayMetrics().widthPixels;
            winH = a.getResources().getDisplayMetrics().heightPixels;
        }
        settleFoot();
    }

    /// The picture's foot: the 4:3 preview at full width. Held back from the
    /// bottom of the window by the mode row and a finger's room under it, so a
    /// short or wide screen loses picture rather than losing the shutter.
    private static void settleFoot() {
        int want = Math.round(winW / PICTURE_W_OVER_H);
        int most = winH - dp(ROW_MODE + MODE_H) - insetBottom;
        foot = Math.max(dp(200), Math.min(want, most));
    }

    private static int dp(float v) {
        return Math.round(v * density);
    }

    // ---------------------------------------------------------- the window

    /// Build the whole overlay and add it as a window of its own. Main thread.
    private static void build(final Activity activity) {
        FrameLayout root = new FrameLayout(activity);
        // Not transparent: the reference's ground under the picture and around
        // the sheet is the window's own near-black.
        root.setBackgroundColor(GROUND);
        // The camera is modal: nothing behind it may be touched, and a tap on
        // the ground between the controls must not fall through to the app.
        root.setClickable(true);
        root.setFocusable(false);

        // ---- the picture -------------------------------------------------
        // A box of its own so the surface can never spill onto the sheet: a
        // preview that is not quite 4:3 is FITTED inside this rather than
        // cropped past it, because a SurfaceView's surface is placed by its
        // own bounds and would be drawn outside a parent's clip.
        pictureBox = new FrameLayout(activity);
        pictureBox.setClipChildren(true);
        FrameLayout.LayoutParams boxLp =
                new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, foot);
        boxLp.gravity = Gravity.TOP | Gravity.START;
        root.addView(pictureBox, boxLp);

        SurfaceView v = new SurfaceView(activity);
        // NOT setZOrderOnTop and NOT setZOrderMediaOverlay: at the default
        // sublayer the surface sits UNDER this window's own drawing, which is
        // exactly what lets every control paint over it. The window is itself
        // above the app's, so the picture is still on top of the app.
        SurfaceHolder hl = v.getHolder();
        hl.setFixedSize(previewSize.getWidth(), previewSize.getHeight());
        hl.addCallback(new SurfaceHolder.Callback() {
            @Override
            public void surfaceCreated(SurfaceHolder h) {}

            @Override
            public void surfaceChanged(SurfaceHolder h, int fmt, int sw, int sh) {
                surfaceReady = true;
                openDevice();
            }

            @Override
            public void surfaceDestroyed(SurfaceHolder h) {
                surfaceReady = false;
            }
        });
        pictureBox.addView(v, new FrameLayout.LayoutParams(1, 1));
        view = v;
        holder = hl;

        // ---- the picture's rounded bottom edge -----------------------------
        frameView = new FrameView(activity);
        FrameLayout.LayoutParams cornerLp =
                new FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, foot);
        cornerLp.gravity = Gravity.TOP | Gravity.START;
        root.addView(frameView, cornerLp);

        // ---- the gallery sheet ---------------------------------------------
        root.addView(buildSheet(activity), sheetParams());

        // ---- what stands in for a picture that has not arrived ------------
        hint = new TextView(activity);
        hint.setTextColor(0xB3FFFFFF);
        hint.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15f);
        hint.setGravity(Gravity.CENTER);
        hint.setText("Starting the camera…");
        FrameLayout.LayoutParams hlp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, foot);
        hlp.gravity = Gravity.TOP | Gravity.START;
        hlp.leftMargin = dp(32);
        hlp.rightMargin = dp(32);
        root.addView(hint, hlp);

        // ---- the recording pill -------------------------------------------
        LinearLayout rec = new LinearLayout(activity);
        rec.setOrientation(LinearLayout.HORIZONTAL);
        rec.setGravity(Gravity.CENTER_VERTICAL);
        rec.setBackground(pill(SCRIM, dp(14)));
        rec.setPadding(dp(10), 0, dp(12), 0);
        View dot = new View(activity);
        dot.setBackground(pill(REC, dp(4)));
        LinearLayout.LayoutParams dlp = new LinearLayout.LayoutParams(dp(8), dp(8));
        dlp.rightMargin = dp(8);
        rec.addView(dot, dlp);
        recText = new TextView(activity);
        recText.setTextColor(Color.WHITE);
        recText.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 13f);
        recText.setText("0:00");
        rec.addView(recText);
        FrameLayout.LayoutParams rlp =
                new FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, dp(28));
        rlp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        rlp.topMargin = insetTop + dp(ICON_DROP + (ICON_BOX - 28) / 2);
        rec.setVisibility(View.GONE);
        root.addView(rec, rlp);
        recPill = rec;

        // ---- close, top left ----------------------------------------------
        closeBtn = new IconView(activity, IconView.CLOSE);
        FrameLayout.LayoutParams clp =
                new FrameLayout.LayoutParams(dp(ICON_BOX), dp(ICON_BOX));
        clp.gravity = Gravity.TOP | Gravity.START;
        clp.leftMargin = dp(ICON_EDGE);
        clp.topMargin = insetTop + dp(ICON_DROP);
        closeBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                // Not close() — the bridge owns the teardown, and it has a
                // poll running that has to be told to stop first.
                closed = true;
            }
        });
        root.addView(closeBtn, clp);

        // ---- flash, top right ----------------------------------------------
        flashBtn = new IconView(activity, IconView.FLASH_OFF);
        FrameLayout.LayoutParams flp =
                new FrameLayout.LayoutParams(dp(ICON_BOX), dp(ICON_BOX));
        flp.gravity = Gravity.TOP | Gravity.END;
        flp.rightMargin = dp(ICON_EDGE);
        flp.topMargin = insetTop + dp(ICON_DROP);
        flashBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                if (!flashAvailable) return;
                torch(!torch);
                syncUi();
            }
        });
        root.addView(flashBtn, flp);

        // ---- the zoom pill ---------------------------------------------------
        zoomPill = new LinearLayout(activity);
        zoomPill.setOrientation(LinearLayout.HORIZONTAL);
        zoomPill.setGravity(Gravity.CENTER_VERTICAL);
        zoomPill.setBackground(pill(SCRIM, dp(PILL_H / 2)));
        zoomPill.setPadding(dp(PILL_PAD), 0, dp(PILL_PAD), 0);
        chips = new TextView[STOPS.length];
        for (int i = 0; i < STOPS.length; i++) {
            final float stop = STOPS[i];
            TextView c = new TextView(activity);
            c.setText(label(stop));
            c.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15f);
            c.setGravity(Gravity.CENTER);
            c.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    if (!reachable(stop)) return;
                    setZoom(stop);
                    syncUi();
                }
            });
            LinearLayout.LayoutParams lp =
                    new LinearLayout.LayoutParams(dp(CHIP_D), dp(CHIP_D));
            if (i > 0) lp.leftMargin = dp(CHIP_GAP);
            zoomPill.addView(c, lp);
            chips[i] = c;
        }
        root.addView(zoomPill, overFoot(FrameLayout.LayoutParams.WRAP_CONTENT,
                dp(PILL_H), ROW_ZOOM, PILL_H, 0f));

        // ---- the shutter -------------------------------------------------
        shutterBtn = new ShutterView(activity);
        shutterBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                // The one control that is a request, not a command: the
                // bridge answers it so the file lands on the staging page by
                // the route every other attachment takes.
                shutterCount++;
            }
        });
        root.addView(shutterBtn, overFoot(dp(SHUTTER_D), dp(SHUTTER_D),
                ROW_SHUTTER, SHUTTER_D, 0f));

        // ---- the flip ring, beside it -------------------------------------
        flipBtn = new IconView(activity, IconView.FLIP);
        flipBtn.ring = true;
        flipBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                // Not mid-clip: the recorder is bound to the camera that
                // started it.
                if (RECORDING.equals(state)) return;
                flip();
                syncUi();
            }
        });
        root.addView(flipBtn, overFoot(dp(FLIP_D), dp(FLIP_D),
                ROW_SHUTTER, FLIP_D, FLIP_OFFSET));

        // ---- Photo | Video ------------------------------------------------
        modeRow = new LinearLayout(activity);
        modeRow.setOrientation(LinearLayout.HORIZONTAL);
        modeRow.setGravity(Gravity.CENTER_VERTICAL);
        String[] names = { "Photo", "Video" };
        final String[] values = { "photo", "video" };
        modeLabels = new TextView[2];
        for (int i = 0; i < 2; i++) {
            final String value = values[i];
            TextView t = new TextView(activity);
            t.setText(names[i]);
            t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15f);
            t.setGravity(Gravity.CENTER);
            t.setPadding(dp(MODE_PAD), 0, dp(MODE_PAD), 0);
            t.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    // Switching mid-clip would strand the recording.
                    if (RECORDING.equals(state)) return;
                    mode = value;
                    syncUi();
                }
            });
            LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT, dp(MODE_H));
            if (i > 0) lp.leftMargin = dp(MODE_GAP);
            modeRow.addView(t, lp);
            modeLabels[i] = t;
        }
        root.addView(modeRow, overFoot(FrameLayout.LayoutParams.WRAP_CONTENT,
                dp(MODE_H), ROW_MODE, MODE_H, 0f));

        // ---- the window ----------------------------------------------------
        WindowManager.LayoutParams lp = new WindowManager.LayoutParams(
                WindowManager.LayoutParams.MATCH_PARENT,
                WindowManager.LayoutParams.MATCH_PARENT,
                // Sublayer +2: above the activity window and above any
                // SurfaceView the app owns. TYPE_APPLICATION_PANEL (+1) would
                // tie with a setZOrderOnTop SurfaceView and the order would be
                // whatever SurfaceFlinger felt like.
                WindowManager.LayoutParams.TYPE_APPLICATION_SUB_PANEL,
                // NOT_FOCUSABLE keeps the key stream with the activity, which
                // is what lets the app's own Back handling close us (the
                // bridge does it from Slint's close-requested); it implies
                // NOT_TOUCH_MODAL, which is harmless for a full-screen window.
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
                        | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS
                        | WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED,
                PixelFormat.TRANSLUCENT);
        lp.gravity = Gravity.TOP | Gravity.START;
        if (Build.VERSION.SDK_INT >= 28) {
            lp.layoutInDisplayCutoutMode = WindowManager.LayoutParams
                    .LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS;
        }
        try {
            activity.getWindowManager().addView(root, lp);
        } catch (RuntimeException e) {
            fail("the viewfinder could not be shown: " + e);
            return;
        }
        overlay = root;

        // The window's real size is only known once it has been laid out.
        root.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override
            public void onLayoutChange(View x, int l, int t, int r, int b,
                                       int ol, int ot, int or_, int ob) {
                if (r - l <= 0 || b - t <= 0) return;
                if (r - l == winW && b - t == winH) return;
                winW = r - l;
                winH = b - t;
                settleFoot();
                relayout();
            }
        });
        root.setOnApplyWindowInsetsListener(new View.OnApplyWindowInsetsListener() {
            @Override
            public WindowInsets onApplyWindowInsets(View x, WindowInsets wi) {
                int top, bottom;
                if (Build.VERSION.SDK_INT >= 30) {
                    android.graphics.Insets in =
                            wi.getInsets(WindowInsets.Type.systemBars());
                    top = in.top;
                    bottom = in.bottom;
                } else {
                    top = wi.getSystemWindowInsetTop();
                    bottom = wi.getSystemWindowInsetBottom();
                }
                if (top != insetTop || bottom != insetBottom) {
                    insetTop = top;
                    insetBottom = bottom;
                    settleFoot();
                    relayout();
                }
                return wi;
            }
        });

        // The app going to the background gives the sensor back: the bridge
        // sees `closed` on its next pass and tears the rest down.
        try {
            lifecycle = new Lifecycle(activity);
            activity.getApplication().registerActivityLifecycleCallbacks(lifecycle);
        } catch (RuntimeException ignored) {
        }

        applyPreviewBounds();
        syncUi();
        ui.post(TICK);
        loadGallery(activity, 0);
    }

    /// A control standing `above` dp over the picture's foot, `h` tall, and
    /// `offset` dp right of centre. FrameLayout centres horizontally and THEN
    /// adds leftMargin, so the offset is the gap between the two centres.
    private static FrameLayout.LayoutParams overFoot(int w, int h, float above,
                                                     float tall, float offset) {
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(w, h);
        lp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        lp.topMargin = foot - dp(above) - dp(tall / 2f);
        lp.leftMargin = dp(offset);
        return lp;
    }

    /// Everything that moves when the window's size or its insets change.
    private static void relayout() {
        if (overlay == null) return;
        setHeight(pictureBox, foot);
        setHeight(frameView, foot);
        setHeight(hint, foot);
        setTop(closeBtn, insetTop + dp(ICON_DROP));
        setTop(flashBtn, insetTop + dp(ICON_DROP));
        setTop(recPill, insetTop + dp(ICON_DROP + (ICON_BOX - 28) / 2));
        setTop(zoomPill, foot - dp(ROW_ZOOM) - dp(PILL_H / 2));
        setTop(shutterBtn, foot - dp(ROW_SHUTTER) - dp(SHUTTER_D / 2));
        setTop(flipBtn, foot - dp(ROW_SHUTTER) - dp(FLIP_D / 2));
        setTop(modeRow, foot - dp(ROW_MODE) - dp(MODE_H / 2));
        View sheet = galleryScroll == null ? null : (View) galleryScroll.getParent();
        if (sheet != null) sheet.setLayoutParams(sheetParams());
        if (galleryScroll != null) {
            galleryScroll.setPadding(0, 0, 0, insetBottom);
        }
        applyPreviewBounds();
    }

    private static FrameLayout.LayoutParams sheetParams() {
        int top = foot + dp(SHEET_GAP);
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, Math.max(1, winH - top));
        lp.gravity = Gravity.TOP | Gravity.START;
        lp.topMargin = top;
        return lp;
    }

    private static void setTop(View v, int m) {
        if (v == null) return;
        ViewGroup.LayoutParams raw = v.getLayoutParams();
        if (!(raw instanceof FrameLayout.LayoutParams)) return;
        ((FrameLayout.LayoutParams) raw).topMargin = m;
        v.setLayoutParams(raw);
    }

    private static void setHeight(View v, int h) {
        if (v == null) return;
        ViewGroup.LayoutParams raw = v.getLayoutParams();
        if (raw == null) return;
        raw.height = h;
        v.setLayoutParams(raw);
    }

    /// The preview's rectangle inside the picture box: the camera's own shape
    /// FITTED in it. The box is 3:4 and the preview is picked at 4:3, so the
    /// fit is a perfect fill and nothing is lost — and on a device with no
    /// 4:3 preview the thin bars stay INSIDE the box rather than spilling a
    /// surface over the sheet, which a crop would.
    private static void applyPreviewBounds() {
        SurfaceView v = view;
        if (v == null || previewSize == null || winW <= 0 || foot <= 0) return;
        boolean swap = (sensorOrientation % 180) != 0;
        float a = swap
                ? (float) previewSize.getHeight() / (float) previewSize.getWidth()
                : (float) previewSize.getWidth() / (float) previewSize.getHeight();
        int w, h;
        if (a > (float) winW / (float) foot) {
            w = winW;
            h = Math.round(winW / a);
        } else {
            h = foot;
            w = Math.round(foot * a);
        }
        w = Math.max(1, Math.min(w, winW));
        h = Math.max(1, Math.min(h, foot));
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(w, h);
        lp.gravity = Gravity.TOP | Gravity.START;
        lp.leftMargin = (winW - w) / 2;
        lp.topMargin = (foot - h) / 2;
        v.setLayoutParams(lp);
        // The selfie camera shows you a mirror, as every phone's does. Not
        // every SurfaceView honours a view transform; where it does not, the
        // picture is simply un-mirrored, which is what it was before.
        v.setScaleX(front ? -1f : 1f);
    }

    // --------------------------------------------------------- the gallery

    /// One row of the sheet's grid.
    private static final class Shot {
        final Uri uri;
        final String name;
        final long added;
        Bitmap thumb;

        Shot(Uri uri, String name, long added) {
            this.uri = uri;
            this.name = name;
            this.added = added;
        }
    }

    /// The sheet under the picture: the reference's rounded panel, its drag
    /// handle, and the grid.
    private static View buildSheet(Activity activity) {
        LinearLayout sheet = new LinearLayout(activity);
        sheet.setOrientation(LinearLayout.VERTICAL);
        GradientDrawable bg = new GradientDrawable();
        bg.setShape(GradientDrawable.RECTANGLE);
        bg.setColor(SHEET_BG);
        float r = dp(CORNER);
        bg.setCornerRadii(new float[] { r, r, r, r, 0, 0, 0, 0 });
        sheet.setBackground(bg);
        // The sheet swallows its own taps: the ground between thumbnails is
        // not a way through to the app.
        sheet.setClickable(true);

        // The handle band: the pill 21.8dp down, the grid 46dp down.
        FrameLayout head = new FrameLayout(activity);
        View handle = new View(activity);
        handle.setBackground(pill(HANDLE_C, dp(HANDLE_H / 2)));
        FrameLayout.LayoutParams hp =
                new FrameLayout.LayoutParams(dp(HANDLE_W), dp(HANDLE_H));
        hp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        hp.topMargin = dp(HANDLE_TOP);
        head.addView(handle, hp);
        sheet.addView(head, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(SHEET_HEAD)));

        galleryScroll = new ScrollView(activity);
        galleryScroll.setVerticalScrollBarEnabled(false);
        // The last row scrolls clear of the gesture bar instead of resting
        // under it, and the padding does not clip the rows on the way past.
        galleryScroll.setClipToPadding(false);
        galleryScroll.setPadding(0, 0, 0, insetBottom);
        galleryRows = new LinearLayout(activity);
        galleryRows.setOrientation(LinearLayout.VERTICAL);
        galleryScroll.addView(galleryRows, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.WRAP_CONTENT));
        sheet.addView(galleryScroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));
        return sheet;
    }

    /// Read the newest media off MediaStore and fill the grid. `attempt` is
    /// there because the read permission may still be a dialog on screen when
    /// the viewfinder opens: an empty first answer is tried once more a
    /// moment later, and then left alone.
    private static void loadGallery(final Activity a, final int attempt) {
        if (io == null) return;
        io.post(new Runnable() {
            @Override
            public void run() {
                final List<Shot> shots = readGallery(a);
                final Handler h = ui;
                if (h == null) return;
                h.post(new Runnable() {
                    @Override
                    public void run() {
                        fillGallery(a, shots);
                        if (shots.isEmpty() && attempt == 0 && ui != null) {
                            ui.postDelayed(new Runnable() {
                                @Override
                                public void run() {
                                    if (overlay != null) loadGallery(a, 1);
                                }
                            }, 1500);
                        }
                    }
                });
            }
        });
    }

    /// The newest GALLERY_MAX pictures and clips, newest first. A refused read
    /// permission is a SecurityException from the query and an empty sheet —
    /// never a crash.
    private static List<Shot> readGallery(Activity a) {
        List<Shot> out = new ArrayList<>();
        try {
            ContentResolver r = a.getContentResolver();
            collect(r, MediaStore.Images.Media.EXTERNAL_CONTENT_URI, out);
            collect(r, MediaStore.Video.Media.EXTERNAL_CONTENT_URI, out);
        } catch (SecurityException e) {
            return new ArrayList<>();
        } catch (RuntimeException e) {
            return new ArrayList<>();
        }
        Collections.sort(out, new Comparator<Shot>() {
            @Override
            public int compare(Shot x, Shot y) {
                return Long.compare(y.added, x.added);
            }
        });
        while (out.size() > GALLERY_MAX) {
            out.remove(out.size() - 1);
        }
        return out;
    }

    private static void collect(ContentResolver r, Uri base, List<Shot> out) {
        String[] proj = {
                MediaStore.MediaColumns._ID,
                MediaStore.MediaColumns.DISPLAY_NAME,
                MediaStore.MediaColumns.DATE_ADDED,
        };
        // No LIMIT in the sort clause: it is unsupported from API 30 and the
        // cursor is closed after GALLERY_MAX rows anyway.
        Cursor c = r.query(base, proj, null, null,
                MediaStore.MediaColumns.DATE_ADDED + " DESC");
        if (c == null) return;
        try {
            int idCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID);
            int nameCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME);
            int addedCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_ADDED);
            int n = 0;
            while (c.moveToNext() && n < GALLERY_MAX) {
                long id = c.getLong(idCol);
                String name = c.getString(nameCol);
                out.add(new Shot(ContentUris.withAppendedId(base, id),
                        name == null ? "" : name, c.getLong(addedCol)));
                n++;
            }
        } finally {
            c.close();
        }
    }

    /// Lay the thumbnails out: three square cells edge to edge with a 3px
    /// gutter between them and none at the sides, exactly as the reference
    /// does. The reference shows no clip among them, so nothing is badged.
    private static void fillGallery(final Activity a, List<Shot> shots) {
        if (galleryRows == null) return;
        galleryRows.removeAllViews();
        if (shots.isEmpty()) return;
        int gutter = dp(GUTTER);
        int cell = (winW - gutter * (COLUMNS - 1)) / COLUMNS;
        LinearLayout row = null;
        for (int i = 0; i < shots.size(); i++) {
            if (i % COLUMNS == 0) {
                row = new LinearLayout(a);
                row.setOrientation(LinearLayout.HORIZONTAL);
                LinearLayout.LayoutParams rp = new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, cell);
                if (i > 0) rp.topMargin = gutter;
                galleryRows.addView(row, rp);
            }
            final Shot shot = shots.get(i);
            ImageView cellView = new ImageView(a);
            cellView.setScaleType(ImageView.ScaleType.CENTER_CROP);
            cellView.setBackgroundColor(0xFF2B2B28);
            cellView.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    x.setAlpha(0.45f);
                    stagePick(a, shot);
                }
            });
            LinearLayout.LayoutParams cp = new LinearLayout.LayoutParams(cell, cell);
            if (i % COLUMNS != 0) cp.leftMargin = gutter;
            row.addView(cellView, cp);
            thumbnail(a, shot, cellView, cell);
        }
    }

    private static void thumbnail(final Activity a, final Shot shot,
                                  final ImageView into, final int cell) {
        if (io == null) return;
        io.post(new Runnable() {
            @Override
            public void run() {
                Bitmap b = null;
                try {
                    if (Build.VERSION.SDK_INT >= 29) {
                        b = a.getContentResolver().loadThumbnail(
                                shot.uri, new Size(cell, cell), null);
                    } else {
                        b = legacyThumb(a, shot);
                    }
                } catch (Exception ignored) {
                } catch (OutOfMemoryError ignored) {
                }
                final Bitmap done = b;
                if (done == null) return;
                Handler h = ui;
                if (h == null) return;
                h.post(new Runnable() {
                    @Override
                    public void run() {
                        if (overlay == null) return;
                        shot.thumb = done;
                        into.setImageBitmap(done);
                    }
                });
            }
        });
    }

    /// Before API 29 there is no loadThumbnail: MediaStore keeps its own
    /// mini-kind thumbnails instead, one table per medium.
    @SuppressWarnings("deprecation")
    private static Bitmap legacyThumb(Activity a, Shot shot) {
        long id = ContentUris.parseId(shot.uri);
        boolean video = shot.uri.toString()
                .startsWith(MediaStore.Video.Media.EXTERNAL_CONTENT_URI.toString());
        if (video) {
            return MediaStore.Video.Thumbnails.getThumbnail(a.getContentResolver(),
                    id, MediaStore.Video.Thumbnails.MINI_KIND, null);
        }
        return MediaStore.Images.Thumbnails.getThumbnail(a.getContentResolver(),
                id, MediaStore.Images.Thumbnails.MINI_KIND, null);
    }

    /// A tapped thumbnail. The engine's send path wants a real file, not a
    /// content:// URI (SigilFilePicker copies for the same reason), so the
    /// bytes are copied into the directory the bridge handed down and the
    /// path is published for it to stage. It stages by the same call a
    /// gallery pick does, and the staging page arriving is what closes the
    /// viewfinder.
    private static void stagePick(final Activity a, final Shot shot) {
        if (io == null || pickDir.isEmpty()) return;
        io.post(new Runnable() {
            @Override
            public void run() {
                InputStream in = null;
                OutputStream os = null;
                try {
                    File dir = new File(pickDir);
                    if (!dir.mkdirs() && !dir.isDirectory()) return;
                    File out = new File(dir, safeName(a, shot));
                    in = a.getContentResolver().openInputStream(shot.uri);
                    if (in == null) return;
                    os = new FileOutputStream(out);
                    byte[] buf = new byte[64 * 1024];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        os.write(buf, 0, n);
                    }
                    os.flush();
                    os.close();
                    os = null;
                    if (out.length() == 0) {
                        out.delete();
                        return;
                    }
                    pickedPath = out.getAbsolutePath();
                } catch (Exception e) {
                    failure = "that picture could not be opened";
                } finally {
                    try { if (os != null) os.close(); } catch (Exception ignored) {}
                    try { if (in != null) in.close(); } catch (Exception ignored) {}
                }
            }
        });
    }

    /// The item's own name, made safe to be a filename, with an extension so
    /// the engine can guess the type. Nothing from a provider is trusted as a
    /// path: a separator would write outside the directory we chose.
    private static String safeName(Activity a, Shot shot) {
        String name = shot.name;
        StringBuilder clean = new StringBuilder();
        for (int i = 0; i < name.length(); i++) {
            char c = name.charAt(i);
            clean.append(c < 0x20 || c == '/' || c == '\\' || c == 0x7f ? '_' : c);
        }
        name = clean.toString().trim();
        if (name.isEmpty() || name.equals(".") || name.equals("..")) name = "picture";
        if (name.lastIndexOf('.') <= 0) {
            String ext = MimeTypeMap.getSingleton().getExtensionFromMimeType(
                    a.getContentResolver().getType(shot.uri));
            name = name + "." + (ext == null || ext.isEmpty() ? "jpg" : ext);
        }
        return System.currentTimeMillis() + "-" + name;
    }
    // -------------------------------------------------------- the controls

    /// Every control's look, off the session's state. Runs on the main
    /// thread, on the tick below and after each press.
    private static void syncUi() {
        if (overlay == null) return;
        boolean recording = RECORDING.equals(state);
        boolean live = READY.equals(state) || CAPTURING.equals(state) || recording;

        if (hint != null) {
            if (live) {
                hint.setVisibility(View.GONE);
            } else {
                hint.setVisibility(View.VISIBLE);
                String f = failure;
                hint.setText(ERROR.equals(state) && f != null
                        ? f
                        : "Starting the camera…");
            }
        }

        if (flashBtn != null) {
            flashBtn.kind = torch ? IconView.FLASH_ON : IconView.FLASH_OFF;
            flashBtn.setAlpha(flashAvailable ? 1f : 0.35f);
            flashBtn.invalidate();
        }
        if (flipBtn != null) {
            flipBtn.setAlpha(recording ? 0.35f : 1f);
        }

        for (int i = 0; i < chips.length; i++) {
            boolean on = Math.abs(zoom - STOPS[i]) < 0.001f;
            boolean can = reachable(STOPS[i]);
            chips[i].setBackground(on ? pill(ON_BG, dp(CHIP_D / 2)) : null);
            chips[i].setTextColor(on ? ON_FG : OFF_FG);
            chips[i].setTypeface(null, on ? android.graphics.Typeface.BOLD
                    : android.graphics.Typeface.NORMAL);
            chips[i].setAlpha(can ? 1f : 0.35f);
        }

        for (int i = 0; i < modeLabels.length; i++) {
            boolean on = (i == 0) == "photo".equals(mode);
            modeLabels[i].setBackground(on ? pill(ON_BG, dp(MODE_H / 2)) : null);
            modeLabels[i].setTextColor(on ? ON_FG : OFF_FG);
            modeLabels[i].setTypeface(null, on ? android.graphics.Typeface.BOLD
                    : android.graphics.Typeface.NORMAL);
        }
        // The reference centres the LIT label on the screen and lets the other
        // sit beside it, so the row slides rather than the pill.
        if (modeRow.getWidth() > 0) {
            TextView lit = "photo".equals(mode) ? modeLabels[0] : modeLabels[1];
            float want = modeRow.getWidth() / 2f
                    - (lit.getLeft() + lit.getWidth() / 2f);
            modeRow.setTranslationX(want);
        }

        if (shutterBtn != null) {
            shutterBtn.video = "video".equals(mode);
            shutterBtn.recording = recording;
            shutterBtn.invalidate();
        }
        if (recPill != null) {
            recPill.setVisibility(recording ? View.VISIBLE : View.GONE);
            if (recording && recText != null) {
                long s = Math.max(0, (System.currentTimeMillis() - recordStart) / 1000);
                recText.setText(s / 60 + ":" + (s % 60 < 10 ? "0" : "") + (s % 60));
            }
        }
    }

    /// The overlay's own clock. The session changes state on our camera
    /// thread; this is the cheapest way to let the look follow it without a
    /// listener on every field.
    private static final Runnable TICK = new Runnable() {
        @Override
        public void run() {
            if (overlay == null || ui == null) return;
            syncUi();
            ui.postDelayed(this, 100);
        }
    };

    private static boolean reachable(float stop) {
        return stop >= zoomMin - 0.001f && stop <= zoomMax + 0.001f;
    }

    private static String label(float stop) {
        if (stop == 0.5f) return "0.5";
        if (stop == 1.0f) return "1.0";
        if (stop == 2.0f) return "2.0";
        return String.valueOf(stop);
    }

    private static GradientDrawable pill(int colour, int radius) {
        GradientDrawable d = new GradientDrawable();
        d.setShape(GradientDrawable.RECTANGLE);
        d.setColor(colour);
        d.setCornerRadius(radius);
        return d;
    }

    // ------------------------------------------------------------- drawing

    /// The rounded bottom edge of the picture: black outside a rectangle whose
    /// bottom corners are 28dp round, which is the corner the reference shows
    /// where the viewfinder ends.
    private static final class FrameView extends View {
        private final Paint black = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path cut = new Path();
        private final Path rounded = new Path();

        FrameView(Context c) {
            super(c);
            black.setColor(Color.BLACK);
            black.setStyle(Paint.Style.FILL);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            int w = getWidth();
            int h = getHeight();
            if (w <= 0 || h <= 0) return;
            float r = dp(CORNER);
            cut.reset();
            cut.addRect(0, 0, w, h, Path.Direction.CW);
            rounded.reset();
            rounded.addRoundRect(new RectF(0, 0, w, h),
                    new float[] { 0, 0, 0, 0, r, r, r, r }, Path.Direction.CW);
            cut.op(rounded, Path.Op.DIFFERENCE);
            canvas.drawPath(cut, black);
        }
    }

    /// The shutter: a white ring with a white disc inside it, and in video a
    /// red disc that becomes a rounded square while a clip runs — the same
    /// button ends it.
    private static final class ShutterView extends View {
        boolean video;
        boolean recording;
        private final Paint ring = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);

        ShutterView(Context c) {
            super(c);
            setClickable(true);
            ring.setStyle(Paint.Style.STROKE);
            ring.setColor(Color.WHITE);
            fill.setStyle(Paint.Style.FILL);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float cx = getWidth() / 2f;
            float cy = getHeight() / 2f;
            float stroke = dp(SHUTTER_STROKE);
            ring.setStrokeWidth(stroke);
            canvas.drawCircle(cx, cy, Math.min(cx, cy) - stroke / 2f, ring);

            fill.setColor(video || recording ? REC : Color.WHITE);
            fill.setAlpha(isPressed() ? 180 : 255);
            if (recording) {
                float s = dp(SHUTTER_INNER * 0.62f) / 2f;
                canvas.drawRoundRect(new RectF(cx - s, cy - s, cx + s, cy + s),
                        dp(8), dp(8), fill);
            } else {
                canvas.drawCircle(cx, cy, dp(SHUTTER_INNER) / 2f, fill);
            }
        }

        @Override
        public void setPressed(boolean p) {
            super.setPressed(p);
            invalidate();
        }
    }

    /// The close, the flash and the flip, drawn rather than typeset: the app's
    /// icon font lives inside the Rust binary, where Java cannot reach it, and
    /// these four glyphs are a few lines each.
    private static final class IconView extends View {
        static final int CLOSE = 0;
        static final int FLASH_OFF = 1;
        static final int FLASH_ON = 2;
        static final int FLIP = 3;

        int kind;
        /// The flip carries a ring of its own; the other two are bare.
        boolean ring;

        private final Paint white = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint line = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path path = new Path();
        private final Path tmp = new Path();
        private final RectF box = new RectF();

        IconView(Context c, int kind) {
            super(c);
            this.kind = kind;
            setClickable(true);
            white.setColor(Color.WHITE);
            white.setStyle(Paint.Style.FILL);
            line.setColor(Color.WHITE);
            line.setStyle(Paint.Style.STROKE);
            line.setStrokeCap(Paint.Cap.BUTT);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float cx = getWidth() / 2f;
            float cy = getHeight() / 2f;
            float stroke = dp(STROKE);

            if (ring) {
                line.setStrokeWidth(dp(FLIP_STROKE));
                canvas.drawCircle(cx, cy, Math.min(cx, cy) - dp(FLIP_STROKE) / 2f, line);
            }

            switch (kind) {
                case CLOSE: {
                    float h = dp(CLOSE_GLYPH) / 2f;
                    line.setStrokeWidth(stroke);
                    canvas.drawLine(cx - h, cy - h, cx + h, cy + h, line);
                    canvas.drawLine(cx + h, cy - h, cx - h, cy + h, line);
                    break;
                }
                case FLASH_ON:
                case FLASH_OFF: {
                    float s = dp(FLASH_GLYPH);
                    bolt(cx - s / 2f, cy - s / 2f, s);
                    if (kind == FLASH_OFF) {
                        // The slash, cut clean out of the bolt and then drawn
                        // over the gap — the flash_off glyph's own shape.
                        // ±10dp about the centre: the whole glyph then
                        // measures the reference's 49 × 52 px.
                        float d = s * 0.5f;
                        line.setStrokeWidth(stroke * 2.4f);
                        tmp.reset();
                        tmp.moveTo(cx - d, cy - d);
                        tmp.lineTo(cx + d, cy + d);
                        Path gap = new Path();
                        line.getFillPath(tmp, gap);
                        path.op(gap, Path.Op.DIFFERENCE);
                        line.setStrokeWidth(stroke);
                        Path slash = new Path();
                        line.getFillPath(tmp, slash);
                        path.op(slash, Path.Op.UNION);
                    }
                    canvas.drawPath(path, white);
                    break;
                }
                case FLIP: {
                    // The arcs sit inside the glyph's box; the square brackets
                    // at their ends take the rest of it out to FLIP_GLYPH.
                    float r = dp(FLIP_GLYPH) * 0.40f;
                    float arm = r * 0.55f;
                    line.setStrokeWidth(stroke);
                    box.set(cx - r, cy - r, cx + r, cy + r);
                    // Two half-turns, each ending in the square bracket the
                    // reference draws rather than a triangular arrow head.
                    canvas.drawArc(box, 200, 160, false, line);
                    canvas.drawArc(box, 20, 160, false, line);
                    float ax = cx + (float) (r * Math.cos(Math.toRadians(200)));
                    float ay = cy + (float) (r * Math.sin(Math.toRadians(200)));
                    canvas.drawLine(ax, ay, ax, ay + arm, line);
                    canvas.drawLine(ax - arm, ay + arm, ax + stroke / 2f, ay + arm, line);
                    float bx = cx + (float) (r * Math.cos(Math.toRadians(20)));
                    float by = cy + (float) (r * Math.sin(Math.toRadians(20)));
                    canvas.drawLine(bx, by, bx, by - arm, line);
                    canvas.drawLine(bx - stroke / 2f, by - arm, bx + arm, by - arm, line);
                    break;
                }
                default:
                    break;
            }
        }

        /// Material's own flash bolt, `M7 2v11h3v9l7-12h-4l4-8z`, in a 24-unit
        /// box scaled to `s` at (x, y).
        private void bolt(float x, float y, float s) {
            float u = s / 24f;
            path.reset();
            path.moveTo(x + 7 * u, y + 2 * u);
            path.lineTo(x + 7 * u, y + 13 * u);
            path.lineTo(x + 10 * u, y + 13 * u);
            path.lineTo(x + 10 * u, y + 22 * u);
            path.lineTo(x + 17 * u, y + 10 * u);
            path.lineTo(x + 13 * u, y + 10 * u);
            path.lineTo(x + 17 * u, y + 2 * u);
            path.close();
        }
    }

    /// The activity going to the background gives the sensor back. Only ever
    /// sets the flag: the bridge's poll is what tears the session down, and it
    /// has to see the flag first so it can stop polling.
    private static final class Lifecycle
            implements Application.ActivityLifecycleCallbacks {
        private final Activity mine;

        Lifecycle(Activity a) {
            mine = a;
        }

        @Override public void onActivityCreated(Activity a, Bundle b) {}
        @Override public void onActivityStarted(Activity a) {}
        @Override public void onActivityResumed(Activity a) {}
        @Override public void onActivityPaused(Activity a) {
            if (a == mine) closed = true;
        }
        @Override public void onActivityStopped(Activity a) {
            if (a == mine) closed = true;
        }
        @Override public void onActivitySaveInstanceState(Activity a, Bundle b) {}
        @Override public void onActivityDestroyed(Activity a) {
            if (a == mine) closed = true;
        }
    }

    // -------------------------------------------------------- the session

    /// Read the characteristics of the camera we are about to use and settle
    /// every size off them. Cheap, and safe from any thread, so it happens on
    /// the caller's before a single view or thread is made.
    private static boolean pickCamera(Activity activity) {
        try {
            CameraManager m = (CameraManager) activity.getSystemService(Context.CAMERA_SERVICE);
            if (m == null) return fail("this device has no camera service");
            String chosen = null;
            String fallback = null;
            for (String id : m.getCameraIdList()) {
                CameraCharacteristics c = m.getCameraCharacteristics(id);
                Integer f = c.get(CameraCharacteristics.LENS_FACING);
                if (f == null) continue;
                if (fallback == null) fallback = id;
                boolean isFront = f == CameraMetadata.LENS_FACING_FRONT;
                if (isFront == front) { chosen = id; break; }
            }
            if (chosen == null) chosen = fallback;
            if (chosen == null) return fail("this device has no camera");
            cameraId = chosen;

            CameraCharacteristics c = m.getCameraCharacteristics(cameraId);
            Integer o = c.get(CameraCharacteristics.SENSOR_ORIENTATION);
            sensorOrientation = o == null ? 90 : o;
            Boolean fl = c.get(CameraCharacteristics.FLASH_INFO_AVAILABLE);
            flashAvailable = fl != null && fl;
            activeArray = c.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE);

            // Zoom. From API 30 the logical camera reports a true ratio range,
            // which is the only way to reach an ultra-wide 0.5×; before that
            // there is only digital zoom out of the crop region, whose floor
            // is 1.0.
            zoomMin = 1f;
            zoomMax = 1f;
            zoomRatioSupported = false;
            if (Build.VERSION.SDK_INT >= 30) {
                Range<Float> r = c.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE);
                if (r != null && r.getUpper() > r.getLower()) {
                    zoomMin = r.getLower();
                    zoomMax = r.getUpper();
                    zoomRatioSupported = true;
                }
            }
            if (!zoomRatioSupported) {
                Float max = c.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM);
                zoomMax = max == null ? 1f : Math.max(1f, max);
            }

            StreamConfigurationMap map =
                    c.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
            if (map == null) return fail("this camera reports no output sizes");
            chooseSizes(map);
            return true;
        } catch (CameraAccessException e) {
            return fail("camera: " + e.getMessage());
        } catch (RuntimeException e) {
            return fail("camera: " + e);
        }
    }

    /// The preview size closest in SHAPE to the PICTURE BOX, the biggest JPEG
    /// that shares its aspect, and a recording size at or under 1080p. The box
    /// is 3:4 and every phone's camera offers 4:3, so the match is exact and
    /// applyPreviewBounds has nothing left to fit — which is the whole reason
    /// the reference's picture ends where it does.
    private static void chooseSizes(StreamConfigurationMap map) {
        boolean swap = (sensorOrientation % 180) != 0;
        float want = (float) Math.max(1, winW) / (float) Math.max(1, foot);

        Size best = null;
        float bestDelta = Float.MAX_VALUE;
        long bestArea = 0;
        for (Size s : map.getOutputSizes(SurfaceHolder.class)) {
            if (s.getWidth() > 1920 || s.getHeight() > 1920) continue;
            float shown = swap
                    ? (float) s.getHeight() / (float) s.getWidth()
                    : (float) s.getWidth() / (float) s.getHeight();
            float delta = Math.abs(shown - want);
            long area = (long) s.getWidth() * (long) s.getHeight();
            if (best == null || delta < bestDelta - 0.02f
                    || (Math.abs(delta - bestDelta) <= 0.02f && area > bestArea)) {
                best = s;
                bestDelta = delta;
                bestArea = area;
            }
        }
        previewSize = best != null ? best : new Size(1280, 720);

        float sensorAspect = (float) previewSize.getWidth() / (float) previewSize.getHeight();
        jpegSize = largestWithAspect(map.getOutputSizes(ImageFormat.JPEG), sensorAspect, Integer.MAX_VALUE);
        if (jpegSize == null) jpegSize = previewSize;
        videoSize = largestWithAspect(map.getOutputSizes(MediaRecorder.class), sensorAspect, 1920);
        if (videoSize == null) videoSize = previewSize;
    }

    private static Size largestWithAspect(Size[] sizes, float aspect, int cap) {
        if (sizes == null) return null;
        Size best = null;
        Size anyBest = null;
        for (Size s : sizes) {
            if (s.getWidth() > cap || s.getHeight() > cap) continue;
            if (anyBest == null || area(s) > area(anyBest)) anyBest = s;
            float a = (float) s.getWidth() / (float) s.getHeight();
            if (Math.abs(a - aspect) > 0.05f) continue;
            if (best == null || area(s) > area(best)) best = s;
        }
        return best != null ? best : anyBest;
    }

    private static long area(Size s) {
        return (long) s.getWidth() * (long) s.getHeight();
    }

    private static void openDevice() {
        final Activity a = host;
        if (a == null || bg == null || !surfaceReady || device != null || opening) return;
        opening = true;
        bg.post(new Runnable() {
            @Override
            public void run() {
                try {
                    CameraManager m = (CameraManager) a.getSystemService(Context.CAMERA_SERVICE);
                    if (m == null) { opening = false; fail("this device has no camera service"); return; }
                    m.openCamera(cameraId, new CameraDevice.StateCallback() {
                        @Override
                        public void onOpened(CameraDevice d) {
                            opening = false;
                            device = d;
                            stillSession();
                        }

                        @Override
                        public void onDisconnected(CameraDevice d) {
                            opening = false;
                            d.close();
                            if (device == d) device = null;
                            fail("the camera was taken by something else");
                        }

                        @Override
                        public void onError(CameraDevice d, int error) {
                            opening = false;
                            d.close();
                            if (device == d) device = null;
                            fail("the camera would not open (" + error + ")");
                        }
                    }, bg);
                } catch (CameraAccessException e) {
                    opening = false;
                    fail("camera: " + e.getMessage());
                } catch (SecurityException e) {
                    opening = false;
                    fail("the camera permission was not granted");
                } catch (RuntimeException e) {
                    opening = false;
                    fail("camera: " + e);
                }
            }
        });
    }

    /// Preview + JPEG: the session the page sits in whenever it is not
    /// recording. Also the way back from a recording.
    private static void stillSession() {
        final CameraDevice d = device;
        final SurfaceHolder hl = holder;
        if (d == null || hl == null) return;
        try {
            if (jpeg != null) { jpeg.close(); jpeg = null; }
            jpeg = ImageReader.newInstance(jpegSize.getWidth(), jpegSize.getHeight(),
                    ImageFormat.JPEG, 2);
            jpeg.setOnImageAvailableListener(new ImageReader.OnImageAvailableListener() {
                @Override
                public void onImageAvailable(ImageReader reader) {
                    writeJpeg(reader);
                }
            }, bg);

            List<Surface> outputs = new ArrayList<>(2);
            outputs.add(hl.getSurface());
            outputs.add(jpeg.getSurface());
            d.createCaptureSession(outputs, new CameraCaptureSession.StateCallback() {
                @Override
                public void onConfigured(CameraCaptureSession s) {
                    session = s;
                    startPreview(CameraDevice.TEMPLATE_PREVIEW, null);
                    state = READY;
                }

                @Override
                public void onConfigureFailed(CameraCaptureSession s) {
                    fail("the camera preview could not be set up");
                }
            }, bg);
        } catch (CameraAccessException e) {
            fail("camera: " + e.getMessage());
        } catch (RuntimeException e) {
            fail("camera: " + e);
        }
    }

    /// The repeating request behind the picture. `extra` is the recorder's
    /// surface while a clip is running, and null otherwise.
    private static void startPreview(int template, Surface extra) {
        CameraDevice d = device;
        CameraCaptureSession s = session;
        SurfaceHolder hl = holder;
        if (d == null || s == null || hl == null) return;
        try {
            CaptureRequest.Builder b = d.createCaptureRequest(template);
            b.addTarget(hl.getSurface());
            if (extra != null) b.addTarget(extra);
            b.set(CaptureRequest.CONTROL_AF_MODE,
                    CameraMetadata.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
            applyControls(b);
            s.setRepeatingRequest(b.build(), null, bg);
        } catch (CameraAccessException e) {
            fail("camera: " + e.getMessage());
        } catch (RuntimeException e) {
            fail("camera: " + e);
        }
    }

    /// Zoom and torch, on whatever request is being built. Both are per-request
    /// settings in Camera2, so they ride every one of them.
    private static void applyControls(CaptureRequest.Builder b) {
        float z = Math.max(zoomMin, Math.min(zoomMax, zoom));
        if (zoomRatioSupported && Build.VERSION.SDK_INT >= 30) {
            b.set(CaptureRequest.CONTROL_ZOOM_RATIO, z);
        } else if (activeArray != null) {
            z = Math.max(1f, z);
            int cw = Math.max(1, Math.round(activeArray.width() / z));
            int ch = Math.max(1, Math.round(activeArray.height() / z));
            int cx = activeArray.left + (activeArray.width() - cw) / 2;
            int cy = activeArray.top + (activeArray.height() - ch) / 2;
            b.set(CaptureRequest.SCALER_CROP_REGION, new Rect(cx, cy, cx + cw, cy + ch));
        }
        if (flashAvailable) {
            b.set(CaptureRequest.CONTROL_AE_MODE, CameraMetadata.CONTROL_AE_MODE_ON);
            b.set(CaptureRequest.FLASH_MODE,
                    torch ? CameraMetadata.FLASH_MODE_TORCH : CameraMetadata.FLASH_MODE_OFF);
        }
    }

    // ------------------------------------------------------------ the page

    /// The other way round. The device has to be closed and opened again —
    /// there is one CameraDevice per camera — but the view and its surface
    /// stay, so the picture comes back in place rather than blinking away.
    public static void flip() {
        final Activity a = host;
        if (a == null || bg == null) return;
        final boolean want = !front;
        state = OPENING;
        bg.post(new Runnable() {
            @Override
            public void run() {
                closeCamera();
                front = want;
                if (!pickCamera(a)) return;
                a.runOnUiThread(new Runnable() {
                    @Override
                    public void run() {
                        SurfaceHolder hl = holder;
                        if (hl == null || view == null) return;
                        hl.setFixedSize(previewSize.getWidth(), previewSize.getHeight());
                        applyPreviewBounds();
                        // The other lens has its own range; a stop it cannot
                        // reach must not stay lit.
                        zoom = 1f;
                        torch = false;
                        openDevice();
                    }
                });
            }
        });
    }

    /// The zoom chips. Clamped to what the device answered with; a device with
    /// no ultra-wide simply never reports a minimum under 1.
    public static void setZoom(final float ratio) {
        zoom = Math.max(zoomMin, Math.min(zoomMax, ratio));
        reapply();
    }

    /// The flash toggle: a torch on the preview, which is also what lights a
    /// still, since the shot is taken out of the running preview.
    public static void torch(final boolean on) {
        torch = on && flashAvailable;
        reapply();
    }

    private static void reapply() {
        if (bg == null) return;
        bg.post(new Runnable() {
            @Override
            public void run() {
                if (RECORDING.equals(state) && recorder != null) {
                    startPreview(CameraDevice.TEMPLATE_RECORD, recorder.getSurface());
                } else {
                    startPreview(CameraDevice.TEMPLATE_PREVIEW, null);
                }
            }
        });
    }

    // ------------------------------------------------------------ capturing

    /// One JPEG, written to `path`. state() goes to "capturing" and back to
    /// "ready" with lastPath() set once the file is on disk.
    public static void capture(final String path) {
        if (bg == null || session == null || device == null || jpeg == null) {
            fail("the camera is not ready");
            return;
        }
        pendingPhoto = path;
        state = CAPTURING;
        bg.post(new Runnable() {
            @Override
            public void run() {
                CameraDevice d = device;
                CameraCaptureSession s = session;
                ImageReader r = jpeg;
                if (d == null || s == null || r == null) { fail("the camera is not ready"); return; }
                try {
                    CaptureRequest.Builder b =
                            d.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE);
                    b.addTarget(r.getSurface());
                    b.set(CaptureRequest.CONTROL_AF_MODE,
                            CameraMetadata.CONTROL_AF_MODE_CONTINUOUS_PICTURE);
                    // The sensor is mounted sideways on every phone; without
                    // this the file is a landscape picture with an upright
                    // scene in it, and every viewer shows it on its side.
                    b.set(CaptureRequest.JPEG_ORIENTATION, jpegOrientation());
                    applyControls(b);
                    s.capture(b.build(), null, bg);
                } catch (CameraAccessException e) {
                    fail("camera: " + e.getMessage());
                } catch (RuntimeException e) {
                    fail("camera: " + e);
                }
            }
        });
    }

    private static void writeJpeg(ImageReader reader) {
        String path = pendingPhoto;
        Image img = null;
        try {
            img = reader.acquireLatestImage();
            if (img == null || path.isEmpty()) return;
            ByteBuffer buf = img.getPlanes()[0].getBuffer();
            byte[] bytes = new byte[buf.remaining()];
            buf.get(bytes);
            FileOutputStream out = new FileOutputStream(path);
            try {
                out.write(bytes);
            } finally {
                out.close();
            }
            lastPath = path;
            pendingPhoto = "";
            state = READY;
        } catch (Exception e) {
            fail("the photo could not be written: " + e);
        } finally {
            if (img != null) img.close();
        }
    }

    /// Start recording to `path`. The session is rebuilt around the
    /// recorder's surface; creating a session replaces the still one.
    public static void startVideo(final String path) {
        if (bg == null || device == null) {
            fail("the camera is not ready");
            return;
        }
        if (RECORDING.equals(state)) return;
        pendingVideo = path;
        bg.post(new Runnable() {
            @Override
            public void run() {
                final CameraDevice d = device;
                final SurfaceHolder hl = holder;
                if (d == null || hl == null) { fail("the camera is not ready"); return; }
                // With the microphone if it is ours to use, and silently
                // without it if the grant was refused — a silent clip beats no
                // clip, and platform.rs asks for both permissions together.
                MediaRecorder r = buildRecorder(path, true);
                if (r == null) r = buildRecorder(path, false);
                if (r == null) return;
                recorder = r;
                final Surface rs = r.getSurface();
                try {
                    List<Surface> outputs = new ArrayList<>(2);
                    outputs.add(hl.getSurface());
                    outputs.add(rs);
                    d.createCaptureSession(outputs, new CameraCaptureSession.StateCallback() {
                        @Override
                        public void onConfigured(CameraCaptureSession s) {
                            session = s;
                            startPreview(CameraDevice.TEMPLATE_RECORD, rs);
                            try {
                                MediaRecorder mr = recorder;
                                if (mr == null) return;
                                mr.start();
                                recordStart = System.currentTimeMillis();
                                state = RECORDING;
                            } catch (RuntimeException e) {
                                fail("recording would not start: " + e);
                            }
                        }

                        @Override
                        public void onConfigureFailed(CameraCaptureSession s) {
                            fail("this camera cannot record video");
                        }
                    }, bg);
                } catch (CameraAccessException e) {
                    fail("camera: " + e.getMessage());
                } catch (RuntimeException e) {
                    fail("camera: " + e);
                }
            }
        });
    }

    private static MediaRecorder buildRecorder(String path, boolean audio) {
        MediaRecorder r = new MediaRecorder();
        try {
            if (audio) r.setAudioSource(MediaRecorder.AudioSource.CAMCORDER);
            r.setVideoSource(MediaRecorder.VideoSource.SURFACE);
            r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
            r.setOutputFile(path);
            r.setVideoEncodingBitRate(8_000_000);
            r.setVideoFrameRate(30);
            r.setVideoSize(videoSize.getWidth(), videoSize.getHeight());
            r.setVideoEncoder(MediaRecorder.VideoEncoder.H264);
            if (audio) r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC);
            // The same turn as the JPEG's: a clip is written landscape with a
            // rotation on it, which every player honours.
            r.setOrientationHint(jpegOrientation());
            r.prepare();
            return r;
        } catch (Exception e) {
            try { r.reset(); } catch (RuntimeException ignored) {}
            try { r.release(); } catch (RuntimeException ignored) {}
            if (!audio) fail("recording could not be set up: " + e);
            return null;
        }
    }

    /// Stop the clip and go back to the still session. lastPath() is the file
    /// once state() is "ready" again.
    public static void stopVideo() {
        if (bg == null || !RECORDING.equals(state)) return;
        bg.post(new Runnable() {
            @Override
            public void run() {
                MediaRecorder r = recorder;
                recorder = null;
                String path = pendingVideo;
                pendingVideo = "";
                boolean written = false;
                if (r != null) {
                    try {
                        r.stop();
                        written = true;
                    } catch (RuntimeException e) {
                        // Under about a second MediaRecorder refuses to stop
                        // and the file it leaves is unplayable.
                        failure = "that clip was too short";
                    }
                    try { r.reset(); } catch (RuntimeException ignored) {}
                    try { r.release(); } catch (RuntimeException ignored) {}
                }
                if (written) lastPath = path;
                state = READY;
                stillSession();
            }
        });
    }

    /// The turn to put on a file, from the sensor's mounting and the way the
    /// phone is being held. The front camera is mirrored, so its correction
    /// runs the other way.
    private static int jpegOrientation() {
        int d = 0;
        try {
            int r = host.getWindowManager().getDefaultDisplay().getRotation();
            if (r == Surface.ROTATION_90) d = 90;
            else if (r == Surface.ROTATION_180) d = 180;
            else if (r == Surface.ROTATION_270) d = 270;
        } catch (RuntimeException ignored) {
        }
        if (front) return (360 - ((sensorOrientation + d) % 360)) % 360;
        return (sensorOrientation - d + 360) % 360;
    }

    // -------------------------------------------------------------- asking

    /// idle | opening | ready | capturing | recording | error.
    public static String state() {
        return state;
    }

    /// The last file written, or "".
    public static String lastPath() {
        return lastPath;
    }

    public static float zoomMin() {
        return zoomMin;
    }

    public static float zoomMax() {
        return zoomMax;
    }

    public static boolean hasFlash() {
        return flashAvailable;
    }

    public static boolean isFront() {
        return front;
    }

    /// "photo" | "video" — which shutter the overlay is showing. The bridge
    /// reads it to know which capture to run, and remembers it for the next
    /// time the viewfinder opens.
    public static String mode() {
        return mode;
    }

    /// How many times the shutter has been pressed since the viewfinder
    /// opened — open() zeroes it, so a press left over from the session before
    /// cannot read as a fresh one. The bridge keeps the last number it saw; a
    /// change is one press.
    public static int shutterCount() {
        return shutterCount;
    }

    /// The X was pressed, or the activity went away. The bridge closes on it.
    public static boolean closed() {
        return closed;
    }

    /// A thumbnail in the gallery sheet was tapped and its bytes are now a
    /// file at this path, or "". The bridge stages it exactly as it stages a
    /// gallery pick, and the staging page arriving closes the viewfinder.
    public static String pickedPath() {
        return pickedPath;
    }

    /// The last error, or null. It is NOT cleared by reading: two callers ask
    /// (the page's poll, which shows it, and a capture waiting on a file), and
    /// whichever asked first would otherwise take it from the other. It is
    /// cleared by the next open().
    public static String failure() {
        return failure;
    }

    // ------------------------------------------------------------- closing

    /// Stop everything and take the window away. Safe to call twice, and safe
    /// to call when nothing was ever opened.
    public static void close() {
        final Activity a = host;
        final Handler h = bg;
        final HandlerThread t = thread;
        // Every static is taken out HERE, on the caller's thread, and only the
        // slow part is posted. open() closes first, so a teardown that reached
        // for the statics later could close the camera the next open had just
        // started — the page being left and entered again in one breath.
        final MediaRecorder r = recorder;
        final CameraCaptureSession s = session;
        final CameraDevice d = device;
        final ImageReader j = jpeg;
        final HandlerThread iot = ioThread;
        host = null;
        bg = null;
        thread = null;
        io = null;
        ioThread = null;
        recorder = null;
        session = null;
        device = null;
        jpeg = null;
        opening = false;
        state = IDLE;
        pendingPhoto = "";
        pendingVideo = "";
        pickedPath = "";
        // The MediaStore thread holds nothing the camera needs; it is only
        // ever reading a cursor or a bitmap, and letting it finish that read
        // costs nothing.
        if (iot != null) iot.quitSafely();
        if (a != null) {
            a.runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    dropView(a);
                }
            });
        }
        if (h == null) {
            shutDown(r, s, d, j);
            return;
        }
        h.post(new Runnable() {
            @Override
            public void run() {
                shutDown(r, s, d, j);
                if (t != null) t.quitSafely();
            }
        });
    }

    /// The camera half of closing, on our own thread.
    private static void closeCamera() {
        MediaRecorder r = recorder;
        CameraCaptureSession s = session;
        CameraDevice d = device;
        ImageReader j = jpeg;
        recorder = null;
        session = null;
        device = null;
        jpeg = null;
        opening = false;
        shutDown(r, s, d, j);
    }

    /// Let go of everything, in the order that keeps the framework happy: the
    /// recorder before the session that feeds it, the session before the
    /// device that owns it.
    private static void shutDown(MediaRecorder r, CameraCaptureSession s,
                                 CameraDevice d, ImageReader j) {
        if (r != null) {
            try { r.stop(); } catch (RuntimeException ignored) {}
            try { r.reset(); } catch (RuntimeException ignored) {}
            try { r.release(); } catch (RuntimeException ignored) {}
        }
        if (s != null) {
            try { s.stopRepeating(); } catch (Exception ignored) {}
            try { s.close(); } catch (RuntimeException ignored) {}
        }
        if (d != null) {
            try { d.close(); } catch (RuntimeException ignored) {}
        }
        if (j != null) {
            try { j.close(); } catch (RuntimeException ignored) {}
        }
    }

    /// Take the overlay window down. Main thread.
    private static void dropView(Activity a) {
        if (ui != null) ui.removeCallbacks(TICK);
        ViewGroup o = overlay;
        overlay = null;
        pictureBox = null;
        view = null;
        holder = null;
        frameView = null;
        galleryScroll = null;
        galleryRows = null;
        hint = null;
        recPill = null;
        recText = null;
        closeBtn = null;
        flashBtn = null;
        flipBtn = null;
        shutterBtn = null;
        zoomPill = null;
        chips = null;
        modeRow = null;
        modeLabels = null;
        surfaceReady = false;
        Application.ActivityLifecycleCallbacks lc = lifecycle;
        lifecycle = null;
        if (lc != null) {
            try {
                a.getApplication().unregisterActivityLifecycleCallbacks(lc);
            } catch (RuntimeException ignored) {
            }
        }
        if (o == null) return;
        try {
            a.getWindowManager().removeViewImmediate(o);
        } catch (RuntimeException ignored) {
        }
    }

    private static boolean fail(String why) {
        failure = why;
        state = ERROR;
        return false;
    }
}
