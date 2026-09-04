// The camera, seen by the phone: Google Messages' viewfinder, measured off it
// and rebuilt as views.
//
// WHY THIS IS A WINDOW OF ITS OWN. The app is a NativeActivity: it calls
// Window.takeSurface() and paints the activity window's surface itself,
// through EGL, from Rust. Two things follow, and between them they decide the
// whole shape of this file.
//
//   * Ordinary Android views added to the activity (addContentView) are laid
//     out but never drawn — ViewRootImpl does not own that surface any more.
//     So a TextView over the activity would be invisible.
//   * A SurfaceView gets a surface of its own, so it IS drawn. But its
//     z-order is a SUBLAYER of the activity window: setZOrderMediaOverlay is
//     sublayer −1, BELOW the window the app paints, so the camera would vanish
//     behind the app; only setZOrderOnTop (+1) is above it, and then nothing
//     the app draws can ever be on top of the picture.
//
// The way out is a second WINDOW, added to the WindowManager as
// TYPE_APPLICATION_SUB_PANEL — sublayer +2, above the activity window AND
// above any of its SurfaceViews — with the activity's own token. That window
// is an ordinary one: its ViewRootImpl draws it, so ordinary views work again,
// and the preview inside it is a plain SurfaceView at the DEFAULT z, which
// punches a transparent hole through the window's surface for every sibling
// added after it to paint over.
//
// THE COMPOSITION IS MEASURED, NOT GUESSED. Every number in the `dp` block
// below was taken off Google Messages on this same phone with ImageMagick, at
// 1344×2992 and density 408 (1dp = 2.55px); the px figure is in the comment
// beside each. Two of them decide everything else:
//
//   * The picture is the 4:3 preview AT FULL WIDTH. 1344 × 4/3 = 1792, and
//     1792 is exactly where the reference's picture ends. Every control hangs
//     off that foot (mode 32, shutter 112, zoom 195 above it).
//   * The close and the flash are centred 281px down — an ABSOLUTE 110dp from
//     the top of the screen, NOT an inset plus a margin. Hanging them off the
//     status-bar inset put them 66px too low.
//
// THE ICONS ARE THE APP'S OWN GLYPHS. Hand-drawn paths are never identical;
// these are Material Symbols codepoints out of ui/icons.slint (close E5CD,
// flash_off E3E6, flash_on E3E7, flip_camera_android EA37), drawn from the
// very font the Slint side bundles. Java cannot read a font out of the Rust
// binary, so platform.rs writes it to the app's cache once and hands the path
// down. Each glyph is turned into a Path and scaled so its INK measures
// exactly what the reference's does, which no choice of text size can promise.
//
//     overlay window (SUB_PANEL, translucent, no limits, over the cutout)
//     └── FrameLayout ................ the ground; eats every touch
//         ├── pictureBox ............. W × W·4/3, clips its child
//         │   └── SurfaceView ........ the preview, fitted (4:3 in 3:4 = fill)
//         ├── FrameView .............. the picture's 28dp rounded bottom edge
//         ├── header ................. chevron + "To …", shown when expanded
//         ├── SheetView .............. the gallery; drags between rest and full
//         │   ├── head ............... the handle ⇄ search field + Photos |
//         │   │                        Collections, cross-faded on the drag
//         │   └── ScrollView ......... 3 columns, month headers, buckets
//         ├── TextView ............... "Starting the camera…" / the failure
//         ├── recording pill ......... red dot + elapsed, top centre
//         ├── IconView(CLOSE) / IconView(FLASH) ....... 110dp down
//         ├── zoom pill .............. 0.5 / 1.0 / 2.0, the lit disc SLIDING
//         ├── ShutterView ............ the ring; its disc crossfades to red
//         ├── IconView(FLIP) ......... the flip ring beside it
//         └── mode row ............... Photo | Video, the row SLIDING so the
//                                      chosen label comes to centre
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
// Everything here is static and called from the engine's threads: every touch
// of a view hops to the main thread, every camera call hops to a HandlerThread
// of ours, every MediaStore read hops to a second one, and every question the
// bridge asks reads a volatile field.

import android.animation.ValueAnimator;
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
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.Typeface;
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
import android.util.Log;
import android.util.Range;
import android.util.Size;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.VelocityTracker;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.animation.DecelerateInterpolator;
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
import java.util.Calendar;
import java.util.Collections;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class SigilCamera {
    /// What state() answers. The bridge shows the first three and treats
    /// "error" as "close and toast".
    public static final String IDLE = "idle";
    public static final String OPENING = "opening";
    public static final String READY = "ready";
    public static final String CAPTURING = "capturing";
    public static final String RECORDING = "recording";
    public static final String ERROR = "error";

    private static final String TAG = "sigil";

    // ------------------------------------------------------- the reference
    //
    // Google Messages on this phone: Screenshot_20260904-130122 (viewfinder)
    // and -130128 (the gallery dragged full). 1344×2992, density 408.

    /// The picture is the 4:3 preview at full width, so the box is 3:4.
    /// 1344 × 4/3 = 1792, and the reference's picture ends at exactly 1792.
    private static final float PICTURE_W_OVER_H = 3f / 4f;

    /// The close and the flash: 35×35px and 49×52px of ink, both centred
    /// 73px in from their edge and 281px DOWN THE SCREEN. Absolute, not off
    /// the status-bar inset — hanging them off the inset put them at 347.
    private static final float ICON_BOX = 48f;
    private static final float ICON_CX = 28f;        // 71px from the edge
    private static final float ICON_CY = 110f;       // 281px from the top
    private static final float CLOSE_INK = 13.7f;    // 35px
    private static final float FLASH_INK = 20.4f;    // 52px, the taller side

    /// The shutter: outer ring Ø 194px, 8px stroke, a 17-18px gap, a 143px
    /// disc. Ours measured 194 / 8 / 18 / 142 — within a pixel, so unchanged.
    private static final float SHUTTER_D = 76f;      // 194px
    private static final float SHUTTER_STROKE = 3.1f;// 8px
    private static final float SHUTTER_INNER = 56f;  // 143px
    /// The flip ring: Ø 133px, 6px stroke, its glyph 66px across.
    private static final float FLIP_D = 52f;         // 133px
    private static final float FLIP_STROKE = 2.35f;  // 6px
    private static final float FLIP_INK = 25.9f;     // 66px
    /// The zoom pill: 341×79px. The chip block is what is centred on screen,
    /// not the pill — 13px of pad on the left and none on the right, which is
    /// what puts the chips on 548.5 / 671.5 / 794.5 with the pill at 495..835.
    private static final float PILL_H = 31f;         // 79px
    private static final float CHIP_D = 32f;         // 82px
    private static final float CHIP_GAP = 16f;       // centres 122.75px apart
    private static final float PILL_PAD_L = 5.1f;    // 13px; the right is 0
    /// Photo | Video: two 180×67px cells 18px apart, so their centres are
    /// 198px apart, and the LIT one is centred on screen — which is what the
    /// reference shows, not a centred row.
    private static final float MODE_CELL = 70.5f;    // 180px
    private static final float MODE_GAP = 7f;        // 18px
    private static final float MODE_H = 26.3f;       // 67px
    /// Every drawn line that is not a glyph.
    private static final float STROKE = 2.4f;        // 6px
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

    /// The gallery sheet AT REST. Foot 1792 → ground to 1832 → sheet from
    /// 1833, handle centred at 1888.5, grid from 1951.
    private static final float SHEET_GAP = 16f;      // 41px
    private static final float HANDLE_W = 32f;       // 82px
    private static final float HANDLE_H = 4f;        // 10px
    private static final float HANDLE_TOP = 19.8f;   // pill top; centre 21.8
    private static final float SHEET_HEAD = 46f;     // 118px, sheet top → grid
    /// The grid: three square cells edge to edge with a 3px gutter between
    /// them and none at the sides. Cells measured 446 × 446px in both states.
    private static final float GUTTER = 1.2f;        // 3px
    private static final int COLUMNS = 3;
    /// Twenty rows: enough for the month bands to have something to divide,
    /// and few enough that the bitmaps behind them stay small. A cell is
    /// 446px, so a full-size thumbnail is 796 KB and sixty of them would be
    /// 48 MB of heap — they are decoded at two thirds instead, which is 21 MB
    /// and still sharper than the grid can show.
    private static final int GALLERY_MAX = 60;
    private static final float THUMB_SCALE = 2f / 3f;

    /// The gallery sheet DRAGGED FULL (-130128). The panel's top is 363; the
    /// chevron and the title ride above it on the ground, on the same 281px
    /// line as the close and the flash.
    private static final float EX_TOP = 142.4f;      // 363px
    private static final float TITLE_X = 55f;        // 141px
    private static final float TITLE_SP = 18f;       // ink 34px tall
    private static final float SEARCH_TOP = 51.8f;   // 495 − 363 = 132px
    private static final float SEARCH_H = 56.5f;     // 144px (495..638)
    private static final float SEARCH_SIDE = 19.6f;  // 50px
    private static final float SEARCH_ICON = 13.7f;  // 35px of ink
    private static final float SEARCH_ICON_X = 45f;  // 115px from the screen
    private static final float SEARCH_TEXT_X = 72f;  // 184px from the screen
    private static final float SEARCH_SP = 16f;
    private static final float CHIPS_TOP = 127.8f;   // 689 − 363 = 326px
    private static final float CHIP2_H = 40f;        // 102px (689..790)
    private static final float CHIP2_SIDE = 11.8f;   // 30px
    private static final float CHIP2_GAP = 7.8f;     // 20px (662..681)
    private static final float CHIP2_SP = 16f;
    private static final float CHIP2_ICON = 11f;     // 28px of ink
    private static final float GRID_TOP_EX = 198f;   // 868 − 363 = 505px
    private static final float MONTH_H = 52f;        // 132px band
    private static final float MONTH_X = 18f;        // 46px
    private static final float MONTH_SP = 15f;
    /// The head is 46dp at rest and 198dp full; the sheet's top travels from
    /// foot+16dp to 142.4dp.
    private static final long SNAP_MS = 260L;
    private static final long MODE_MS = 250L;
    private static final long FLASH_MS = 200L;
    private static final long ZOOM_MS = 300L;

    /// Colours, sampled off the reference.
    private static final int ON_BG = 0xFF9F9E97;     // the lit chip / mode pill
    private static final int ON_FG = 0xFF20211C;
    private static final int OFF_FG = 0xFFE5E5E5;
    private static final int SCRIM = 0x73000000;     // the zoom pill
    private static final int REC = 0xFFE0403A;
    private static final int GROUND = 0xFF0E0E0D;    // under the picture
    private static final int SHEET_BG = 0xFF20201D;
    private static final int HANDLE_C = 0xFF585753;
    private static final int FIELD_BG = 0xFF0E0E0D;  // the search field
    private static final int FIELD_FG = 0xFFACABA5;
    private static final int CHIP2_ON = 0xFFC6C8B8;
    private static final int CHIP2_ON_FG = 0xFF262622;
    private static final int CHIP2_OFF = 0xFF3E3D39;
    private static final int CHIP2_OFF_FG = 0xFFE3E2DC;
    private static final int MONTH_FG = 0xFFD2D1CB;
    private static final int TITLE_FG = 0xFFE5E1E6;
    private static final int CELL_BG = 0xFF2B2B28;

    /// Material Symbols codepoints, the same ones ui/icons.slint names.
    private static final String G_CLOSE     = "\uE5CD"; // close
    private static final String G_FLASH_OFF = "\uE3E6"; // flash_off
    private static final String G_FLASH_ON  = "\uE3E7"; // flash_on
    private static final String G_FLIP      = "\uEA37"; // flip_camera_android
    private static final String G_SEARCH    = "\uEF7A"; // search
    private static final String G_CHEVRON   = "\uE5CF"; // expand_more
    private static final String G_PHOTO     = "\uE3F4"; // image
    private static final String G_ALBUM     = "\uE413"; // photo_library

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
    private static FrameLayout zoomPill;
    private static View zoomLit;
    private static TextView[] chips;
    private static LinearLayout modeRow;
    private static TextView[] modeLabels;
    private static View cameraLayer;
    private static SheetView sheet;
    private static LinearLayout sheetHead;
    private static View handleBand;
    private static View searchBand;
    private static ScrollView galleryScroll;
    private static LinearLayout galleryRows;
    private static View header;
    private static View[] tabChips;
    private static Typeface symbols;
    private static Application.ActivityLifecycleCallbacks lifecycle;

    private static ValueAnimator modeAnim;
    private static ValueAnimator zoomAnim;
    private static ValueAnimator sheetAnim;

    private static CameraDevice device;
    private static CameraCaptureSession session;
    private static ImageReader jpeg;
    private static MediaRecorder recorder;
    /// Kept alive so a zoom frame is one set() and one setRepeatingRequest
    /// rather than a whole rebuild sixty times a second.
    private static CaptureRequest.Builder repeating;

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
    /// the picture's foot — the line the controls are measured from.
    private static int winW, winH, insetTop, insetBottom, foot;
    private static String pickDir = "";
    private static String title = "";

    private static volatile boolean surfaceReady;
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
    private static volatile String pendingPhoto = "";
    private static volatile String pendingVideo = "";

    /// The three things the overlay tells the bridge rather than doing itself.
    private static volatile int shutterCount;
    private static volatile boolean closed;
    private static volatile String pickedPath = "";

    private static long recordStart;
    /// 0 at rest, 1 dragged full. Everything the sheet moves reads it.
    private static float expand;
    /// Which bucket the grid is showing, or "" for everything.
    private static String bucket = "";
    private static String bucketName = "";

    private SigilCamera() {}

    // ------------------------------------------------------------- opening

    /// Put the viewfinder up over everything. `facing` is "front" or anything
    /// else for the back camera; `startMode` is "video" or anything else;
    /// `dir` is where a tapped gallery item is copied to; `fontPath` is the
    /// Material Symbols file the bridge wrote out of the binary; `to` is the
    /// room's display name for the expanded sheet's title. A second call
    /// replaces the first.
    public static void open(final Activity activity, final String facing,
                            final String startMode, final String dir,
                            final String fontPath, final String to) {
        close();
        host = activity;
        front = "front".equals(facing);
        mode = "video".equals(startMode) ? "video" : "photo";
        pickDir = dir == null ? "" : dir;
        title = to == null ? "" : to;
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
        expand = 0f;
        bucket = "";
        bucketName = "";

        symbols = null;
        if (fontPath != null && !fontPath.isEmpty()) {
            try {
                symbols = Typeface.createFromFile(fontPath);
            } catch (RuntimeException e) {
                Log.w(TAG, "camera: the icon font would not load: " + e);
            }
        }

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
    /// the picture's foot off them.
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
        // what lets every control paint over it.
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

        // ---- everything that is the CAMERA, in one layer ------------------
        // Held together so the drag can fade the lot out as the sheet rises:
        // the reference shows nothing but black above a sheet that is full.
        FrameLayout cam = new FrameLayout(activity);
        cameraLayer = cam;
        root.addView(cam, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));
        buildControls(activity, cam);

        // ---- the gallery sheet ---------------------------------------------
        sheet = new SheetView(activity);
        buildSheet(activity, sheet);
        root.addView(sheet, sheetParams());

        // ---- the expanded sheet's own title row ----------------------------
        root.addView(buildHeader(activity), headerParams());

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
                // bridge does it from Slint's close-requested).
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

        try {
            lifecycle = new Lifecycle(activity);
            activity.getApplication().registerActivityLifecycleCallbacks(lifecycle);
        } catch (RuntimeException ignored) {
        }

        applyPreviewBounds();
        applyExpand(0f);
        syncUi();
        ui.post(TICK);
        loadGallery(activity, 0);
    }

    /// The close, the flash, the zoom pill, the shutter, the flip and the
    /// mode row: everything that belongs to the camera rather than the sheet.
    private static void buildControls(final Activity activity, FrameLayout cam) {
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
        cam.addView(hint, hlp);

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
        rlp.topMargin = dp(ICON_CY) - dp(14);
        rec.setVisibility(View.GONE);
        cam.addView(rec, rlp);
        recPill = rec;

        // ---- close and flash, on the 110dp line ----------------------------
        closeBtn = new IconView(activity, G_CLOSE, CLOSE_INK);
        closeBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                // Not close() — the bridge owns the teardown, and it has a
                // poll running that has to be told to stop first.
                closed = true;
            }
        });
        cam.addView(closeBtn, iconParams(true));

        flashBtn = new IconView(activity, G_FLASH_OFF, FLASH_INK);
        flashBtn.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                if (!flashAvailable || "video".equals(mode)) return;
                torch(!torch);
                syncUi();
            }
        });
        cam.addView(flashBtn, iconParams(false));

        // ---- the zoom pill ---------------------------------------------------
        // The lit disc is a view of its own BEHIND the chips, so choosing a
        // stop slides it rather than repainting three backgrounds.
        zoomPill = new FrameLayout(activity);
        zoomPill.setBackground(pill(SCRIM, dp(PILL_H / 2)));
        // The lit disc is 82px in a 79px pill — 1.5px proud top and bottom,
        // exactly as the reference has it — so the pill must not clip.
        zoomPill.setClipChildren(false);
        zoomPill.setClipToPadding(false);
        zoomLit = new View(activity);
        zoomLit.setBackground(pill(ON_BG, dp(CHIP_D / 2)));
        FrameLayout.LayoutParams litLp =
                new FrameLayout.LayoutParams(dp(CHIP_D), dp(CHIP_D));
        litLp.gravity = Gravity.TOP | Gravity.START;
        litLp.topMargin = (dp(PILL_H) - dp(CHIP_D)) / 2;
        zoomPill.addView(zoomLit, litLp);
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
                    glideZoom(stop);
                }
            });
            FrameLayout.LayoutParams lp =
                    new FrameLayout.LayoutParams(dp(CHIP_D), dp(CHIP_D));
            lp.gravity = Gravity.TOP | Gravity.START;
            lp.leftMargin = chipX(i);
            lp.topMargin = (dp(PILL_H) - dp(CHIP_D)) / 2;
            zoomPill.addView(c, lp);
            chips[i] = c;
        }
        cam.addView(zoomPill, pillParams());

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
        cam.addView(shutterBtn, overFoot(dp(SHUTTER_D), dp(SHUTTER_D),
                ROW_SHUTTER, SHUTTER_D, 0f));

        // ---- the flip ring, beside it -------------------------------------
        flipBtn = new IconView(activity, G_FLIP, FLIP_INK);
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
        cam.addView(flipBtn, overFoot(dp(FLIP_D), dp(FLIP_D),
                ROW_SHUTTER, FLIP_D, FLIP_OFFSET));

        // ---- Photo | Video ------------------------------------------------
        // Two cells of the SAME width 18px apart, so their centres are 198px
        // apart whichever is lit; the row then slides so the lit one lands on
        // the middle of the screen, which is what the reference shows.
        modeRow = new LinearLayout(activity);
        modeRow.setOrientation(LinearLayout.HORIZONTAL);
        String[] names = { "Photo", "Video" };
        final String[] values = { "photo", "video" };
        modeLabels = new TextView[2];
        for (int i = 0; i < 2; i++) {
            final String value = values[i];
            TextView t = new TextView(activity);
            t.setText(names[i]);
            t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 15f);
            t.setGravity(Gravity.CENTER);
            t.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    // Switching mid-clip would strand the recording.
                    if (RECORDING.equals(state)) return;
                    if (value.equals(mode)) return;
                    mode = value;
                    glideMode();
                }
            });
            LinearLayout.LayoutParams lp =
                    new LinearLayout.LayoutParams(dp(MODE_CELL), dp(MODE_H));
            if (i > 0) lp.leftMargin = dp(MODE_GAP);
            modeRow.addView(t, lp);
            modeLabels[i] = t;
        }
        FrameLayout.LayoutParams mlp = new FrameLayout.LayoutParams(
                dp(MODE_CELL) * 2 + dp(MODE_GAP), dp(MODE_H));
        mlp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        mlp.topMargin = foot - dp(ROW_MODE) - dp(MODE_H) / 2;
        cam.addView(modeRow, mlp);
    }

    /// The close (left) or the flash (right), 71px in and centred 281px down.
    private static FrameLayout.LayoutParams iconParams(boolean left) {
        FrameLayout.LayoutParams lp =
                new FrameLayout.LayoutParams(dp(ICON_BOX), dp(ICON_BOX));
        lp.gravity = Gravity.TOP | (left ? Gravity.START : Gravity.END);
        int edge = dp(ICON_CX) - dp(ICON_BOX) / 2;
        if (left) lp.leftMargin = edge; else lp.rightMargin = edge;
        lp.topMargin = dp(ICON_CY) - dp(ICON_BOX) / 2;
        return lp;
    }

    /// The zoom pill. What is centred on the screen is the CHIP BLOCK, not the
    /// pill: the reference pads 13px on the left of the first chip and nothing
    /// on the right of the last, so the pill hangs 6px left of centre.
    private static FrameLayout.LayoutParams pillParams() {
        int block = dp(CHIP_D) * 3 + dp(CHIP_GAP) * 2;
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                block + dp(PILL_PAD_L), dp(PILL_H));
        lp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        lp.leftMargin = -dp(PILL_PAD_L) / 2;
        lp.topMargin = foot - dp(ROW_ZOOM) - dp(PILL_H) / 2;
        return lp;
    }

    private static int chipX(int i) {
        return dp(PILL_PAD_L) + i * (dp(CHIP_D) + dp(CHIP_GAP));
    }

    /// A control standing `above` dp over the picture's foot, `tall` dp high,
    /// and `offset` dp right of centre. FrameLayout centres horizontally and
    /// THEN adds leftMargin, so the offset is the gap between the two centres.
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
        setTop(recPill, dp(ICON_CY) - dp(14));
        setTop(closeBtn, dp(ICON_CY) - dp(ICON_BOX) / 2);
        setTop(flashBtn, dp(ICON_CY) - dp(ICON_BOX) / 2);
        setTop(zoomPill, foot - dp(ROW_ZOOM) - dp(PILL_H) / 2);
        setTop(shutterBtn, foot - dp(ROW_SHUTTER) - dp(SHUTTER_D) / 2);
        setTop(flipBtn, foot - dp(ROW_SHUTTER) - dp(FLIP_D) / 2);
        setTop(modeRow, foot - dp(ROW_MODE) - dp(MODE_H) / 2);
        if (sheet != null) sheet.setLayoutParams(sheetParams());
        if (header != null) header.setLayoutParams(headerParams());
        if (galleryScroll != null) galleryScroll.setPadding(0, 0, 0, insetBottom);
        applyPreviewBounds();
        applyExpand(expand);
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
        // The selfie camera shows you a mirror, as every phone's does.
        v.setScaleX(front ? -1f : 1f);
    }

    // ----------------------------------------------------- the animations

    /// Photo ⇄ Video. The row slides so the chosen label comes to the middle
    /// of the screen, and the shutter's disc crossfades to red over the same
    /// curve — one animation, two things moving, as the reference does.
    private static void glideMode() {
        if (modeRow == null) return;
        if (modeAnim != null) modeAnim.cancel();
        final float from = modeRow.getTranslationX();
        final float to = modeRest();
        final boolean video = "video".equals(mode);
        modeAnim = ValueAnimator.ofFloat(0f, 1f);
        modeAnim.setDuration(MODE_MS);
        modeAnim.setInterpolator(new DecelerateInterpolator());
        modeAnim.addUpdateListener(new ValueAnimator.AnimatorUpdateListener() {
            @Override
            public void onAnimationUpdate(ValueAnimator a) {
                float t = (Float) a.getAnimatedValue();
                if (modeRow != null) modeRow.setTranslationX(from + (to - from) * t);
                if (shutterBtn != null) {
                    shutterBtn.red = video ? t : 1f - t;
                    shutterBtn.invalidate();
                }
                // The flash has nothing to do with a clip, so it goes away
                // for Video and comes back for Photo.
                if (flashBtn != null) {
                    flashBtn.setAlpha(video ? 1f - t : t * flashRest());
                }
            }
        });
        modeAnim.start();
        syncModeLabels();
    }

    /// Where the row has to sit for the lit label to be centred on screen.
    private static float modeRest() {
        int cell = dp(MODE_CELL);
        int gap = dp(MODE_GAP);
        int rowW = cell * 2 + gap;
        // The row is centred, so the lit cell's own centre is already
        // (rowW/2 ∓ (cell+gap)/2) off the middle; undo that.
        float lit = "video".equals(mode) ? cell + gap + cell / 2f : cell / 2f;
        return rowW / 2f - lit;
    }

    private static float flashRest() {
        if ("video".equals(mode)) return 0f;
        return flashAvailable ? 1f : 0.35f;
    }

    /// A zoom stop, reached SMOOTHLY: the ratio is animated and the repeating
    /// request re-issued on every frame, so the viewfinder travels between
    /// stops instead of jumping. The lit disc slides under the chips beside it.
    private static void glideZoom(final float target) {
        final float want = Math.max(zoomMin, Math.min(zoomMax, target));
        if (zoomAnim != null) zoomAnim.cancel();
        final float from = zoom;
        final float litFrom = zoomLit == null ? 0f : zoomLit.getTranslationX();
        final float litTo = chipX(nearestStop(target)) - chipX(0);
        zoomAnim = ValueAnimator.ofFloat(0f, 1f);
        zoomAnim.setDuration(ZOOM_MS);
        zoomAnim.setInterpolator(new DecelerateInterpolator());
        zoomAnim.addUpdateListener(new ValueAnimator.AnimatorUpdateListener() {
            @Override
            public void onAnimationUpdate(ValueAnimator a) {
                float t = (Float) a.getAnimatedValue();
                zoom = from + (want - from) * t;
                pushZoom();
                if (zoomLit != null) {
                    zoomLit.setTranslationX(litFrom + (litTo - litFrom) * t);
                }
            }
        });
        zoomAnim.start();
        zoomTarget = target;
        syncChips();
    }

    private static volatile float zoomTarget = 1f;

    private static int nearestStop(float v) {
        int best = 0;
        for (int i = 1; i < STOPS.length; i++) {
            if (Math.abs(STOPS[i] - v) < Math.abs(STOPS[best] - v)) best = i;
        }
        return best;
    }

    /// One zoom frame: set the ratio on the request already built and hand it
    /// back to the session. Rebuilding a request per frame would be far more
    /// work than this, which is why `repeating` is kept alive.
    private static void pushZoom() {
        final Handler h = bg;
        if (h == null) return;
        h.post(new Runnable() {
            @Override
            public void run() {
                CameraCaptureSession s = session;
                CaptureRequest.Builder b = repeating;
                if (s == null || b == null) return;
                try {
                    applyControls(b);
                    s.setRepeatingRequest(b.build(), null, bg);
                } catch (CameraAccessException | RuntimeException ignored) {
                }
            }
        });
    }

    // --------------------------------------------------------- the controls

    /// Every control's look, off the session's state. Main thread, on the
    /// tick below and after each press. Anything that ANIMATES is driven from
    /// its own animator instead, so this never fights one.
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
                hint.setText(ERROR.equals(state) && f != null ? f : "Starting the camera…");
            }
        }

        if (flashBtn != null) {
            flashBtn.glyph = torch ? G_FLASH_ON : G_FLASH_OFF;
            if (modeAnim == null || !modeAnim.isRunning()) {
                flashBtn.setAlpha(flashRest());
            }
            flashBtn.invalidate();
        }
        if (flipBtn != null) flipBtn.setAlpha(recording ? 0.35f : 1f);

        syncChips();
        syncModeLabels();
        if (modeRow != null && (modeAnim == null || !modeAnim.isRunning())) {
            modeRow.setTranslationX(modeRest());
        }

        if (shutterBtn != null) {
            if (modeAnim == null || !modeAnim.isRunning()) {
                shutterBtn.red = "video".equals(mode) ? 1f : 0f;
            }
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

    private static void syncChips() {
        if (chips == null) return;
        int lit = nearestStop(zoomTarget);
        for (int i = 0; i < chips.length; i++) {
            boolean on = i == lit;
            boolean can = reachable(STOPS[i]);
            chips[i].setTextColor(on ? ON_FG : OFF_FG);
            chips[i].setTypeface(null, on ? Typeface.BOLD : Typeface.NORMAL);
            chips[i].setAlpha(can ? 1f : 0.35f);
        }
        if (zoomLit != null && (zoomAnim == null || !zoomAnim.isRunning())) {
            zoomLit.setTranslationX(chipX(lit) - chipX(0));
        }
    }

    private static void syncModeLabels() {
        if (modeLabels == null) return;
        for (int i = 0; i < modeLabels.length; i++) {
            boolean on = (i == 0) == "photo".equals(mode);
            modeLabels[i].setBackground(on ? pill(ON_BG, dp(MODE_H / 2)) : null);
            modeLabels[i].setTextColor(on ? ON_FG : OFF_FG);
            modeLabels[i].setTypeface(null, on ? Typeface.BOLD : Typeface.NORMAL);
        }
    }

    /// The overlay's own clock: the session changes state on our camera
    /// thread, and this is the cheapest way to let the look follow it.
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

    // ----------------------------------------------------------- the sheet

    /// The gallery panel. It lives at its FULL height and travels by
    /// translationY between rest (its top at the picture's foot + 16dp) and
    /// full (its top at 142.4dp, where the reference puts it), following the
    /// finger and snapping on release.
    private static final class SheetView extends FrameLayout {
        private final int slop;
        private float y0;
        private boolean dragging;
        private float startExpand;
        private VelocityTracker tracker;

        SheetView(Context c) {
            super(c);
            slop = ViewConfiguration.get(c).getScaledTouchSlop();
            setClickable(true);
        }

        @Override
        public boolean onInterceptTouchEvent(MotionEvent ev) {
            switch (ev.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    y0 = ev.getY();
                    dragging = false;
                    return false;
                case MotionEvent.ACTION_MOVE: {
                    float dy = ev.getY() - y0;
                    if (Math.abs(dy) < slop) return false;
                    boolean up = dy < 0;
                    // Rising: any upward drag takes the sheet with it.
                    if (up && expand < 1f) return start(ev);
                    // Falling: only from a grid that is already at its top,
                    // so a scrolled grid keeps its own gesture.
                    if (!up && expand > 0f
                            && (galleryScroll == null || galleryScroll.getScrollY() == 0)) {
                        return start(ev);
                    }
                    return false;
                }
                default:
                    return false;
            }
        }

        private boolean start(MotionEvent ev) {
            dragging = true;
            startExpand = expand;
            y0 = ev.getY();
            if (sheetAnim != null) sheetAnim.cancel();
            tracker = VelocityTracker.obtain();
            tracker.addMovement(ev);
            return true;
        }

        @Override
        public boolean onTouchEvent(MotionEvent ev) {
            if (!dragging) {
                if (ev.getActionMasked() == MotionEvent.ACTION_DOWN) {
                    y0 = ev.getY();
                    return true;
                }
                if (ev.getActionMasked() == MotionEvent.ACTION_MOVE
                        && Math.abs(ev.getY() - y0) >= slop) {
                    start(ev);
                } else {
                    return true;
                }
            }
            if (tracker != null) tracker.addMovement(ev);
            float travel = Math.max(1, restTop() - dp(EX_TOP));
            switch (ev.getActionMasked()) {
                case MotionEvent.ACTION_MOVE: {
                    float dy = ev.getY() - y0;
                    applyExpand(clamp(startExpand - dy / travel));
                    return true;
                }
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL: {
                    float v = 0f;
                    if (tracker != null) {
                        tracker.computeCurrentVelocity(1000);
                        v = tracker.getYVelocity();
                        tracker.recycle();
                        tracker = null;
                    }
                    // A flick decides on its own; a slow drag on where it got.
                    boolean full = Math.abs(v) > dp(600)
                            ? v < 0
                            : expand > 0.5f;
                    snap(full ? 1f : 0f);
                    dragging = false;
                    return true;
                }
                default:
                    return true;
            }
        }
    }

    private static float clamp(float v) {
        return v < 0f ? 0f : (v > 1f ? 1f : v);
    }

    /// The sheet's top when it is at rest: 16dp under the picture's foot.
    private static int restTop() {
        return foot + dp(SHEET_GAP);
    }

    private static FrameLayout.LayoutParams sheetParams() {
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                Math.max(1, winH - dp(EX_TOP)));
        lp.gravity = Gravity.TOP | Gravity.START;
        lp.topMargin = dp(EX_TOP);
        return lp;
    }

    private static FrameLayout.LayoutParams headerParams() {
        FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, dp(ICON_BOX));
        lp.gravity = Gravity.TOP | Gravity.START;
        lp.topMargin = dp(ICON_CY) - dp(ICON_BOX) / 2;
        return lp;
    }

    /// Everything the drag moves, from one number.
    private static void applyExpand(float t) {
        expand = t;
        if (sheet == null) return;
        float travel = restTop() - dp(EX_TOP);
        sheet.setTranslationY(travel * (1f - t));
        // The head grows from the handle band into the search field and the
        // two tabs, and the two cross-fade through each other.
        if (sheetHead != null) {
            int h = Math.round(dp(SHEET_HEAD) + (dp(GRID_TOP_EX) - dp(SHEET_HEAD)) * t);
            ViewGroup.LayoutParams lp = sheetHead.getLayoutParams();
            if (lp != null && lp.height != h) {
                lp.height = h;
                sheetHead.setLayoutParams(lp);
            }
        }
        if (handleBand != null) handleBand.setAlpha(clamp(1f - t * 2f));
        if (searchBand != null) {
            searchBand.setAlpha(clamp(t * 2f - 1f));
            searchBand.setVisibility(t > 0.5f ? View.VISIBLE : View.INVISIBLE);
        }
        if (header != null) {
            header.setAlpha(clamp(t * 2f - 1f));
            header.setVisibility(t > 0.5f ? View.VISIBLE : View.INVISIBLE);
        }
        // The reference shows nothing but black above a sheet that is up. A
        // SurfaceView ignores alpha, so the picture is taken away outright the
        // moment the drag begins; the controls, being ordinary views, fade.
        if (cameraLayer != null) cameraLayer.setAlpha(clamp(1f - t * 1.6f));
        if (pictureBox != null) {
            pictureBox.setVisibility(t > 0.02f ? View.INVISIBLE : View.VISIBLE);
        }
        if (frameView != null) frameView.setAlpha(clamp(1f - t * 4f));
    }

    private static void snap(final float to) {
        if (sheetAnim != null) sheetAnim.cancel();
        final float from = expand;
        if (Math.abs(to - from) < 0.001f) {
            applyExpand(to);
            return;
        }
        sheetAnim = ValueAnimator.ofFloat(0f, 1f);
        sheetAnim.setDuration(SNAP_MS);
        sheetAnim.setInterpolator(new DecelerateInterpolator());
        sheetAnim.addUpdateListener(new ValueAnimator.AnimatorUpdateListener() {
            @Override
            public void onAnimationUpdate(ValueAnimator a) {
                float t = (Float) a.getAnimatedValue();
                applyExpand(from + (to - from) * t);
            }
        });
        sheetAnim.start();
    }

    /// The row that rides above the sheet when it is full: a chevron that puts
    /// it back down, and who the picture is going to.
    private static View buildHeader(Activity a) {
        FrameLayout row = new FrameLayout(a);
        IconView chev = new IconView(a, G_CHEVRON, CLOSE_INK);
        chev.setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View x) {
                snap(0f);
            }
        });
        FrameLayout.LayoutParams clp =
                new FrameLayout.LayoutParams(dp(ICON_BOX), dp(ICON_BOX));
        clp.gravity = Gravity.TOP | Gravity.START;
        clp.leftMargin = dp(ICON_CX) - dp(ICON_BOX) / 2;
        row.addView(chev, clp);

        TextView t = new TextView(a);
        // The recipient's own name, handed down at open(): nothing about who
        // is being written to belongs in this file.
        t.setText(title.isEmpty() ? "To" : "To " + title);
        t.setTextColor(TITLE_FG);
        t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, TITLE_SP);
        t.setSingleLine(true);
        t.setEllipsize(android.text.TextUtils.TruncateAt.END);
        t.setGravity(Gravity.CENTER_VERTICAL);
        FrameLayout.LayoutParams tlp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, dp(ICON_BOX));
        tlp.gravity = Gravity.TOP | Gravity.START;
        tlp.leftMargin = dp(TITLE_X);
        tlp.rightMargin = dp(24);
        row.addView(t, tlp);
        row.setVisibility(View.INVISIBLE);
        header = row;
        return row;
    }

    /// The sheet's own contents: the head (handle at rest, search field and
    /// tabs when full) over the scrolling grid.
    private static void buildSheet(Activity activity, FrameLayout sheetRoot) {
        GradientDrawable bg = new GradientDrawable();
        bg.setShape(GradientDrawable.RECTANGLE);
        bg.setColor(SHEET_BG);
        float r = dp(CORNER);
        bg.setCornerRadii(new float[] { r, r, r, r, 0, 0, 0, 0 });
        sheetRoot.setBackground(bg);

        LinearLayout column = new LinearLayout(activity);
        column.setOrientation(LinearLayout.VERTICAL);
        sheetRoot.addView(column, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));

        sheetHead = new LinearLayout(activity);
        FrameLayout head = new FrameLayout(activity);
        sheetHead.addView(head, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT,
                LinearLayout.LayoutParams.MATCH_PARENT));
        column.addView(sheetHead, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(SHEET_HEAD)));

        // The handle: 82 × 10px, its centre 21.8dp below the sheet's top.
        View handle = new View(activity);
        handle.setBackground(pill(HANDLE_C, dp(HANDLE_H / 2)));
        FrameLayout.LayoutParams hp =
                new FrameLayout.LayoutParams(dp(HANDLE_W), dp(HANDLE_H));
        hp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        hp.topMargin = dp(HANDLE_TOP);
        head.addView(handle, hp);
        handleBand = handle;

        // The search field and the two tabs, which only exist when the sheet
        // is full. Ours has no Google Photos behind it, so it says what it is.
        FrameLayout band = new FrameLayout(activity);
        head.addView(band, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));
        band.setVisibility(View.INVISIBLE);
        searchBand = band;

        FrameLayout field = new FrameLayout(activity);
        field.setBackground(pill(FIELD_BG, dp(SEARCH_H / 2)));
        FrameLayout.LayoutParams flp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, dp(SEARCH_H));
        flp.gravity = Gravity.TOP | Gravity.START;
        flp.leftMargin = dp(SEARCH_SIDE);
        flp.rightMargin = dp(SEARCH_SIDE);
        flp.topMargin = dp(SEARCH_TOP);
        band.addView(field, flp);

        IconView mag = new IconView(activity, G_SEARCH, SEARCH_ICON);
        mag.tint = FIELD_FG;
        FrameLayout.LayoutParams mlp =
                new FrameLayout.LayoutParams(dp(ICON_BOX), dp(SEARCH_H));
        mlp.gravity = Gravity.TOP | Gravity.START;
        mlp.leftMargin = dp(SEARCH_ICON_X - SEARCH_SIDE) - dp(ICON_BOX) / 2;
        field.addView(mag, mlp);

        TextView ph = new TextView(activity);
        ph.setText("Search photos");
        ph.setTextColor(FIELD_FG);
        ph.setTextSize(TypedValue.COMPLEX_UNIT_DIP, SEARCH_SP);
        ph.setGravity(Gravity.CENTER_VERTICAL);
        FrameLayout.LayoutParams plp = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT, dp(SEARCH_H));
        plp.gravity = Gravity.TOP | Gravity.START;
        plp.leftMargin = dp(SEARCH_TEXT_X - SEARCH_SIDE);
        field.addView(ph, plp);

        // Photos | Collections: two equal chips 20px apart, 30px in from each
        // edge, 102px tall.
        tabChips = new View[2];
        int chipW = (winW - dp(CHIP2_SIDE) * 2 - dp(CHIP2_GAP)) / 2;
        String[] names = { "Photos", "Collections" };
        String[] gl = { G_PHOTO, G_ALBUM };
        for (int i = 0; i < 2; i++) {
            final boolean photos = i == 0;
            LinearLayout chip = new LinearLayout(activity);
            chip.setOrientation(LinearLayout.HORIZONTAL);
            chip.setGravity(Gravity.CENTER);
            IconView ic = new IconView(activity, gl[i], CHIP2_ICON);
            LinearLayout.LayoutParams ilp =
                    new LinearLayout.LayoutParams(dp(CHIP2_ICON * 1.8f), dp(CHIP2_H));
            ilp.rightMargin = dp(8);
            chip.addView(ic, ilp);
            TextView lab = new TextView(activity);
            lab.setText(names[i]);
            lab.setTextSize(TypedValue.COMPLEX_UNIT_DIP, CHIP2_SP);
            lab.setGravity(Gravity.CENTER);
            chip.addView(lab, new LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT, dp(CHIP2_H)));
            chip.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    showBuckets = !photos;
                    bucket = "";
                    bucketName = "";
                    syncTabs();
                    Activity a = host;
                    if (a != null) loadGallery(a, 0);
                }
            });
            FrameLayout.LayoutParams clp =
                    new FrameLayout.LayoutParams(chipW, dp(CHIP2_H));
            clp.gravity = Gravity.TOP | Gravity.START;
            clp.leftMargin = dp(CHIP2_SIDE) + i * (chipW + dp(CHIP2_GAP));
            clp.topMargin = dp(CHIPS_TOP);
            band.addView(chip, clp);
            tabChips[i] = chip;
        }
        syncTabs();

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
        column.addView(galleryScroll, new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f));
    }

    private static boolean showBuckets;

    private static void syncTabs() {
        if (tabChips == null) return;
        for (int i = 0; i < tabChips.length; i++) {
            boolean on = (i == 0) != showBuckets;
            tabChips[i].setBackground(pill(on ? CHIP2_ON : CHIP2_OFF, dp(CHIP2_H / 2)));
            LinearLayout row = (LinearLayout) tabChips[i];
            for (int j = 0; j < row.getChildCount(); j++) {
                View c = row.getChildAt(j);
                if (c instanceof TextView) {
                    ((TextView) c).setTextColor(on ? CHIP2_ON_FG : CHIP2_OFF_FG);
                } else if (c instanceof IconView) {
                    ((IconView) c).tint = on ? CHIP2_ON_FG : CHIP2_OFF_FG;
                    c.invalidate();
                }
            }
        }
    }

    // --------------------------------------------------------- the gallery

    /// One item of the grid.
    private static final class Shot {
        final Uri uri;
        final String name;
        final long added;
        final String bucketId;
        final String bucketLabel;

        Shot(Uri uri, String name, long added, String bucketId, String bucketLabel) {
            this.uri = uri;
            this.name = name;
            this.added = added;
            this.bucketId = bucketId;
            this.bucketLabel = bucketLabel;
        }
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
                        if (showBuckets && bucket.isEmpty()) {
                            fillBuckets(a, shots);
                        } else {
                            fillGallery(a, shots);
                        }
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

    /// The newest pictures and clips, newest first. A refused read permission
    /// is a SecurityException from the query and an empty sheet — never a
    /// crash.
    private static List<Shot> readGallery(Activity a) {
        List<Shot> out = new ArrayList<>();
        try {
            ContentResolver r = a.getContentResolver();
            collect(r, MediaStore.Images.Media.EXTERNAL_CONTENT_URI, out);
            collect(r, MediaStore.Video.Media.EXTERNAL_CONTENT_URI, out);
        } catch (RuntimeException e) {
            // A refused read permission arrives as a SecurityException out of
            // the query; an empty sheet is the right answer, not a crash.
            return new ArrayList<>();
        }
        Collections.sort(out, new Comparator<Shot>() {
            @Override
            public int compare(Shot x, Shot y) {
                return Long.compare(y.added, x.added);
            }
        });
        return out;
    }

    private static void collect(ContentResolver r, Uri base, List<Shot> out) {
        String[] proj = {
                MediaStore.MediaColumns._ID,
                MediaStore.MediaColumns.DISPLAY_NAME,
                MediaStore.MediaColumns.DATE_ADDED,
                MediaStore.MediaColumns.BUCKET_ID,
                MediaStore.MediaColumns.BUCKET_DISPLAY_NAME,
        };
        // No LIMIT in the sort clause: it is unsupported from API 30 and the
        // cursor is closed after enough rows anyway.
        Cursor c = r.query(base, proj, null, null,
                MediaStore.MediaColumns.DATE_ADDED + " DESC");
        if (c == null) return;
        try {
            int idCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID);
            int nameCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME);
            int addedCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_ADDED);
            int bidCol = c.getColumnIndex(MediaStore.MediaColumns.BUCKET_ID);
            int blCol = c.getColumnIndex(MediaStore.MediaColumns.BUCKET_DISPLAY_NAME);
            int n = 0;
            while (c.moveToNext() && n < GALLERY_MAX) {
                long id = c.getLong(idCol);
                String name = c.getString(nameCol);
                String bid = bidCol < 0 ? "" : c.getString(bidCol);
                String bl = blCol < 0 ? "" : c.getString(blCol);
                out.add(new Shot(ContentUris.withAppendedId(base, id),
                        name == null ? "" : name, c.getLong(addedCol),
                        bid == null ? "" : bid, bl == null ? "" : bl));
                n++;
            }
        } finally {
            c.close();
        }
    }

    /// Lay the thumbnails out: three square cells edge to edge with a 3px
    /// gutter between them and none at the sides, exactly as the reference
    /// does, with a month band before every group but the newest — which is
    /// how the reference's "September" sits under three unlabelled rows. The
    /// reference shows no clip among its thumbnails, so nothing is badged.
    private static void fillGallery(final Activity a, List<Shot> all) {
        if (galleryRows == null) return;
        galleryRows.removeAllViews();
        List<Shot> shots = new ArrayList<>();
        for (Shot s : all) {
            if (bucket.isEmpty() || bucket.equals(s.bucketId)) shots.add(s);
        }
        if (shots.isEmpty()) return;
        int gutter = dp(GUTTER);
        int cell = (winW - gutter * (COLUMNS - 1)) / COLUMNS;
        LinearLayout row = null;
        // The newest group carries no band — which is why the reference's
        // "September" sits UNDER three unlabelled rows rather than over them.
        String group = month(shots.get(0).added);
        int inRow = 0;
        for (int i = 0; i < shots.size(); i++) {
            final Shot shot = shots.get(i);
            String m = month(shot.added);
            if (!m.equals(group)) {
                group = m;
                row = null;
                inRow = 0;
                galleryRows.addView(monthBand(a, m));
            }
            if (inRow == 0) {
                row = new LinearLayout(a);
                row.setOrientation(LinearLayout.HORIZONTAL);
                LinearLayout.LayoutParams rp = new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT, cell);
                if (galleryRows.getChildCount() > 0
                        && !(galleryRows.getChildAt(galleryRows.getChildCount() - 1)
                                instanceof TextView)) {
                    rp.topMargin = gutter;
                }
                galleryRows.addView(row, rp);
            }
            ImageView cellView = new ImageView(a);
            cellView.setScaleType(ImageView.ScaleType.CENTER_CROP);
            cellView.setBackgroundColor(CELL_BG);
            cellView.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    x.setAlpha(0.45f);
                    stagePick(a, shot);
                }
            });
            LinearLayout.LayoutParams cp = new LinearLayout.LayoutParams(cell, cell);
            if (inRow != 0) cp.leftMargin = gutter;
            row.addView(cellView, cp);
            thumbnail(a, shot, cellView, cell);
            inRow = (inRow + 1) % COLUMNS;
        }
    }

    /// The section header: a 132px band with the month's name 46px in.
    private static TextView monthBand(Activity a, String name) {
        TextView t = new TextView(a);
        t.setText(name);
        t.setTextColor(MONTH_FG);
        t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, MONTH_SP);
        t.setTypeface(null, Typeface.BOLD);
        t.setGravity(Gravity.CENTER_VERTICAL);
        t.setPadding(dp(MONTH_X), 0, dp(MONTH_X), 0);
        t.setLayoutParams(new LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, dp(MONTH_H)));
        return t;
    }

    private static final String[] MONTHS = {
            "January", "February", "March", "April", "May", "June",
            "July", "August", "September", "October", "November", "December" };

    /// DATE_ADDED is in SECONDS since the epoch, not milliseconds.
    private static String month(long addedSeconds) {
        Calendar c = Calendar.getInstance();
        c.setTimeInMillis(addedSeconds * 1000L);
        int m = c.get(Calendar.MONTH);
        int y = c.get(Calendar.YEAR);
        String name = (m >= 0 && m < 12) ? MONTHS[m] : "";
        return y == Calendar.getInstance().get(Calendar.YEAR) ? name : name + " " + y;
    }

    /// The Collections tab: one tile per MediaStore bucket, its newest item as
    /// the cover, its name under it. Tapping one shows that bucket's grid.
    private static void fillBuckets(final Activity a, List<Shot> all) {
        if (galleryRows == null) return;
        galleryRows.removeAllViews();
        Map<String, Shot> covers = new LinkedHashMap<>();
        for (Shot s : all) {
            if (s.bucketId.isEmpty()) continue;
            if (!covers.containsKey(s.bucketId)) covers.put(s.bucketId, s);
        }
        if (covers.isEmpty()) return;
        int gutter = dp(GUTTER);
        int cell = (winW - gutter * (COLUMNS - 1)) / COLUMNS;
        LinearLayout row = null;
        int i = 0;
        for (Map.Entry<String, Shot> e : covers.entrySet()) {
            final Shot cover = e.getValue();
            if (i % COLUMNS == 0) {
                row = new LinearLayout(a);
                row.setOrientation(LinearLayout.HORIZONTAL);
                LinearLayout.LayoutParams rp = new LinearLayout.LayoutParams(
                        LinearLayout.LayoutParams.MATCH_PARENT,
                        cell + dp(MONTH_H) / 2);
                if (i > 0) rp.topMargin = gutter;
                galleryRows.addView(row, rp);
            }
            LinearLayout tile = new LinearLayout(a);
            tile.setOrientation(LinearLayout.VERTICAL);
            ImageView art = new ImageView(a);
            art.setScaleType(ImageView.ScaleType.CENTER_CROP);
            art.setBackgroundColor(CELL_BG);
            tile.addView(art, new LinearLayout.LayoutParams(cell, cell));
            TextView lab = new TextView(a);
            lab.setText(cover.bucketLabel);
            lab.setTextColor(MONTH_FG);
            lab.setTextSize(TypedValue.COMPLEX_UNIT_DIP, 13f);
            lab.setSingleLine(true);
            lab.setEllipsize(android.text.TextUtils.TruncateAt.END);
            lab.setPadding(dp(6), dp(4), dp(6), 0);
            tile.addView(lab, new LinearLayout.LayoutParams(
                    cell, LinearLayout.LayoutParams.WRAP_CONTENT));
            tile.setOnClickListener(new View.OnClickListener() {
                @Override
                public void onClick(View x) {
                    bucket = cover.bucketId;
                    bucketName = cover.bucketLabel;
                    loadGallery(a, 0);
                }
            });
            LinearLayout.LayoutParams cp = new LinearLayout.LayoutParams(
                    cell, LinearLayout.LayoutParams.WRAP_CONTENT);
            if (i % COLUMNS != 0) cp.leftMargin = gutter;
            row.addView(tile, cp);
            thumbnail(a, cover, art, cell);
            i++;
        }
    }

    private static void thumbnail(final Activity a, final Shot shot,
                                  final ImageView into, final int cell) {
        if (io == null) return;
        io.post(new Runnable() {
            @Override
            public void run() {
                Bitmap b = null;
                int want = Math.max(96, Math.round(cell * THUMB_SCALE));
                try {
                    if (Build.VERSION.SDK_INT >= 29) {
                        b = a.getContentResolver().loadThumbnail(
                                shot.uri, new Size(want, want), null);
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
    /// bytes are copied into the directory the bridge handed down and the path
    /// is published for it to stage — by the same call a gallery pick makes.
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

    // ------------------------------------------------------------- drawing

    /// The picture's rounded bottom edge: the ground outside a rectangle whose
    /// bottom corners are 28dp round, which is the corner the reference shows
    /// where the viewfinder ends.
    private static final class FrameView extends View {
        private final Paint ground = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path cut = new Path();
        private final Path rounded = new Path();
        private final RectF box = new RectF();

        FrameView(Context c) {
            super(c);
            ground.setColor(GROUND);
            ground.setStyle(Paint.Style.FILL);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            int w = getWidth();
            int h = getHeight();
            if (w <= 0 || h <= 0) return;
            float r = dp(CORNER);
            box.set(0, 0, w, h);
            cut.reset();
            cut.addRect(box, Path.Direction.CW);
            rounded.reset();
            rounded.addRoundRect(box,
                    new float[] { 0, 0, 0, 0, r, r, r, r }, Path.Direction.CW);
            cut.op(rounded, Path.Op.DIFFERENCE);
            canvas.drawPath(cut, ground);
        }
    }

    /// The shutter: a white ring with a disc inside it. `red` is how far along
    /// the Photo → Video crossfade the disc is, and `recording` squares it off
    /// — the same button ends the clip.
    private static final class ShutterView extends View {
        float red;
        boolean recording;
        private final Paint ring = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint fill = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final RectF box = new RectF();

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

            fill.setColor(mix(Color.WHITE, REC, red));
            fill.setAlpha(isPressed() ? 180 : 255);
            if (recording) {
                float s = dp(SHUTTER_INNER * 0.62f) / 2f;
                box.set(cx - s, cy - s, cx + s, cy + s);
                canvas.drawRoundRect(box, dp(8), dp(8), fill);
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

    private static int mix(int a, int b, float t) {
        t = clamp(t);
        int r = Math.round(Color.red(a) + (Color.red(b) - Color.red(a)) * t);
        int g = Math.round(Color.green(a) + (Color.green(b) - Color.green(a)) * t);
        int bl = Math.round(Color.blue(a) + (Color.blue(b) - Color.blue(a)) * t);
        return Color.argb(255, r, g, bl);
    }

    /// A Material Symbols glyph, drawn from the app's OWN font — the same file
    /// the Slint side bundles, written out of the binary by platform.rs. The
    /// glyph is turned into a Path and scaled so its INK measures exactly the
    /// reference's, which no choice of text size can promise: a font's advance
    /// and its ink are not the same thing, and it was the difference between
    /// them that made the first hand-drawn set the wrong size.
    private static final class IconView extends View {
        String glyph;
        float ink;
        /// The flip carries a ring of its own; nothing else does.
        boolean ring;
        int tint = Color.WHITE;

        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Paint line = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final Path path = new Path();
        private final Path scaled = new Path();
        private final RectF bounds = new RectF();
        private final Matrix matrix = new Matrix();
        private String built = "";

        IconView(Context c, String glyph, float ink) {
            super(c);
            this.glyph = glyph;
            this.ink = ink;
            setClickable(true);
            paint.setStyle(Paint.Style.FILL);
            line.setStyle(Paint.Style.STROKE);
            line.setColor(Color.WHITE);
        }

        @Override
        protected void onDraw(Canvas canvas) {
            float cx = getWidth() / 2f;
            float cy = getHeight() / 2f;

            if (ring) {
                line.setStrokeWidth(dp(FLIP_STROKE));
                line.setColor(tint);
                canvas.drawCircle(cx, cy, Math.min(cx, cy) - dp(FLIP_STROKE) / 2f, line);
            }
            if (symbols == null || glyph == null || glyph.isEmpty()) return;

            if (!glyph.equals(built)) {
                paint.setTypeface(symbols);
                paint.setTextSize(200f);
                path.reset();
                paint.getTextPath(glyph, 0, glyph.length(), 0f, 0f, path);
                built = glyph;
            }
            path.computeBounds(bounds, true);
            if (bounds.width() <= 0 || bounds.height() <= 0) return;
            float s = dp(ink) / Math.max(bounds.width(), bounds.height());
            matrix.reset();
            matrix.setScale(s, s);
            matrix.postTranslate(cx - (bounds.left + bounds.width() / 2f) * s,
                    cy - (bounds.top + bounds.height() / 2f) * s);
            scaled.reset();
            path.transform(matrix, scaled);
            paint.setColor(tint);
            canvas.drawPath(scaled, paint);
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
            // THE ULTRA-WIDE STOP LIVES ON THE LOGICAL CAMERA. Taking the
            // first id with the right facing is what made 0.5× unreachable:
            // a phone can expose its physical lenses beside the logical one,
            // and a physical camera's CONTROL_ZOOM_RATIO_RANGE floors at 1.0
            // — only the logical camera's range reaches below it. So every
            // candidate is weighed and the one that can go WIDEST wins, and
            // each one's range goes into the log so a device that still says
            // 1.0 can be told apart from a bug of ours.
            String chosen = null;
            String fallback = null;
            float widest = Float.MAX_VALUE;
            for (String id : m.getCameraIdList()) {
                CameraCharacteristics c = m.getCameraCharacteristics(id);
                Integer f = c.get(CameraCharacteristics.LENS_FACING);
                if (f == null) continue;
                if (fallback == null) fallback = id;
                boolean isFront = f == CameraMetadata.LENS_FACING_FRONT;
                if (isFront != front) continue;
                float low = 1f;
                float high = 1f;
                if (Build.VERSION.SDK_INT >= 30) {
                    Range<Float> r = c.get(CameraCharacteristics.CONTROL_ZOOM_RATIO_RANGE);
                    if (r != null) { low = r.getLower(); high = r.getUpper(); }
                }
                Log.i(TAG, "camera: id " + id + " facing " + f
                        + " zoom " + low + ".." + high + (logical(c) ? " logical" : ""));
                // A logical camera always beats a physical one, and among
                // equals the wider lens wins.
                float score = logical(c) ? low - 100f : low;
                if (chosen == null || score < widest) {
                    chosen = id;
                    widest = score;
                }
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
            Log.i(TAG, "camera: opening " + cameraId + " zoom "
                    + zoomMin + ".." + zoomMax
                    + (zoomRatioSupported ? " (ratio)" : " (crop)"));
            zoomTarget = 1f;

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

    /// Whether this id is the LOGICAL camera behind several lenses — the only
    /// one whose zoom ratio reaches an ultra-wide.
    private static boolean logical(CameraCharacteristics c) {
        int[] caps = c.get(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES);
        if (caps == null) return false;
        for (int cap : caps) {
            if (cap == CameraMetadata.REQUEST_AVAILABLE_CAPABILITIES_LOGICAL_MULTI_CAMERA) {
                return true;
            }
        }
        return false;
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
            // Kept so a zoom frame is one set() and one setRepeatingRequest
            // rather than a whole rebuild sixty times a second.
            repeating = b;
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
        repeating = null;
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
        repeating = null;
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
