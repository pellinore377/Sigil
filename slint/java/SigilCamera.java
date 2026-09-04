// The camera, seen by the phone.
//
// The app draws everything through one surface of its own and cannot put a
// camera frame on it. The phone can: a SurfaceView laid over the app's own
// surface, fed by a Camera2 preview session, is a live viewfinder standing
// exactly where the attach sheet's camera page would have drawn one — the
// same trick SigilVideo plays for playback, and for the same reason.
//
// The session shape. One CameraDevice and one preview surface, with the
// capture session configured two ways and swapped between them:
//
//   still  — [preview, ImageReader(JPEG)]   TEMPLATE_PREVIEW repeating,
//                                           TEMPLATE_STILL_CAPTURE for a shot
//   record — [preview, MediaRecorder]       TEMPLATE_RECORD repeating
//
// They are two sessions rather than one three-output session because a
// MediaRecorder surface only exists between prepare() and stop(): it has to
// be built for each clip, with that clip's file and orientation on it, and a
// session cannot have an output swapped under it. Creating a session
// replaces the one before it, so the swap is a single call in each direction.
//
// Everything here is static and called from the engine's threads: every touch
// of a view hops to the main thread, every camera call hops to a HandlerThread
// of ours, and every question the bridge asks reads a volatile field. Loaded
// at runtime from the embedded dex (build.rs, platform.rs) alongside
// SigilFilePicker and SigilVideo, so nothing here is named in the manifest —
// the CAMERA permission is, and platform.rs asks for it before calling open().

import android.app.Activity;
import android.content.Context;
import android.graphics.ImageFormat;
import android.graphics.Rect;
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
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.util.Range;
import android.util.Size;
import android.view.Gravity;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;

import java.io.FileOutputStream;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

public final class SigilCamera {
    /// What state() answers. The bridge shows the first three and treats
    /// "error" as "put the page back to the grid with a toast".
    public static final String IDLE = "idle";
    public static final String OPENING = "opening";
    public static final String READY = "ready";
    public static final String CAPTURING = "capturing";
    public static final String RECORDING = "recording";
    public static final String ERROR = "error";

    private static Activity host;
    private static SurfaceView view;
    private static SurfaceHolder holder;
    private static HandlerThread thread;
    private static Handler bg;

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

    /// The rectangle the page asked for, in physical pixels. The view itself
    /// sits at the letterboxed rectangle inside it (see fitted()).
    private static int rx, ry, rw, rh;
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

    private static volatile String state = IDLE;
    private static volatile String lastPath = "";
    private static volatile String failure;
    /// Where the shot or the clip in flight is going.
    private static volatile String pendingPhoto = "";
    private static volatile String pendingVideo = "";

    private SigilCamera() {}

    // ------------------------------------------------------------- opening

    /// Lay a viewfinder over the activity inside (x, y, w, h) — physical
    /// pixels — and start a preview on `facing` ("front" or anything else for
    /// the back camera). A second call replaces the first.
    public static void open(final Activity activity, final int x, final int y,
                            final int w, final int h, final String facing) {
        close();
        host = activity;
        front = "front".equals(facing);
        rx = x; ry = y; rw = Math.max(1, w); rh = Math.max(1, h);
        state = OPENING;
        failure = null;
        lastPath = "";
        pendingPhoto = "";
        pendingVideo = "";
        surfaceReady = false;
        opening = false;
        zoom = 1f;
        torch = false;

        if (!pickCamera(activity)) return;

        thread = new HandlerThread("sigil-camera");
        thread.start();
        bg = new Handler(thread.getLooper());

        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                SurfaceView v = new SurfaceView(activity);
                // Above the app's own surface, which is a SurfaceView too.
                v.setZOrderOnTop(true);
                int[] r = fitted();
                FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(r[2], r[3]);
                lp.gravity = Gravity.TOP | Gravity.START;
                lp.leftMargin = r[0];
                lp.topMargin = r[1];
                SurfaceHolder hl = v.getHolder();
                // The buffer is in sensor coordinates; SurfaceFlinger applies
                // the camera's rotation hint, so the view shows it upright and
                // the view's own rectangle is already the right shape for it.
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
                activity.addContentView(v, lp);
                view = v;
                holder = hl;
            }
        });
    }

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

    /// The preview size closest in SHAPE to the box the page gave us, the
    /// biggest JPEG that shares the preview's own aspect, and a recording
    /// size at or under 1080p. Matching the shape is what keeps the picture
    /// from being stretched; whatever is left over after the match is
    /// letterboxed by fitted(), so the view never distorts either.
    private static void chooseSizes(StreamConfigurationMap map) {
        boolean swap = (sensorOrientation % 180) != 0;
        float want = (float) rw / (float) rh;

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

    /// The view's own rectangle: the preview's shape fitted inside the box the
    /// page asked for, and centred in it. What is left of the box stays as the
    /// page drew it, so a viewfinder is never stretched to fill.
    private static int[] fitted() {
        boolean swap = (sensorOrientation % 180) != 0;
        float a = previewSize == null ? 0.75f
                : (swap ? (float) previewSize.getHeight() / (float) previewSize.getWidth()
                        : (float) previewSize.getWidth() / (float) previewSize.getHeight());
        int w, h;
        if (a > (float) rw / (float) rh) {
            w = rw;
            h = Math.round(rw / a);
        } else {
            h = rh;
            w = Math.round(rh * a);
        }
        w = Math.max(1, Math.min(w, rw));
        h = Math.max(1, Math.min(h, rh));
        return new int[] { rx + (rw - w) / 2, ry + (rh - h) / 2, w, h };
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

    // ------------------------------------------------------------- sessions

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

    /// The page moved or resized the preview box: follow it.
    public static void move(final int x, final int y, final int w, final int h) {
        final Activity a = host;
        rx = x; ry = y; rw = Math.max(1, w); rh = Math.max(1, h);
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                SurfaceView v = view;
                if (v == null) return;
                ViewGroup.LayoutParams raw = v.getLayoutParams();
                if (!(raw instanceof FrameLayout.LayoutParams)) return;
                FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) raw;
                int[] r = fitted();
                lp.leftMargin = r[0];
                lp.topMargin = r[1];
                lp.width = r[2];
                lp.height = r[3];
                v.setLayoutParams(lp);
            }
        });
    }

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
                        SurfaceView v = view;
                        if (hl == null || v == null) return;
                        hl.setFixedSize(previewSize.getWidth(), previewSize.getHeight());
                        ViewGroup.LayoutParams raw = v.getLayoutParams();
                        if (raw instanceof FrameLayout.LayoutParams) {
                            FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) raw;
                            int[] r = fitted();
                            lp.leftMargin = r[0];
                            lp.topMargin = r[1];
                            lp.width = r[2];
                            lp.height = r[3];
                            v.setLayoutParams(lp);
                        }
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

    /// The last error, or null. It is NOT cleared by reading: two callers ask
    /// (the page's poll, which shows it, and a capture waiting on a file), and
    /// whichever asked first would otherwise take it from the other. It is
    /// cleared by the next open().
    public static String failure() {
        return failure;
    }

    // ------------------------------------------------------------- closing

    /// Stop everything and take the view away. Safe to call twice, and safe to
    /// call when nothing was ever opened.
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
        host = null;
        bg = null;
        thread = null;
        recorder = null;
        session = null;
        device = null;
        jpeg = null;
        opening = false;
        state = IDLE;
        pendingPhoto = "";
        pendingVideo = "";
        if (a != null) {
            a.runOnUiThread(new Runnable() {
                @Override
                public void run() {
                    dropView();
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

    private static void dropView() {
        SurfaceView v = view;
        view = null;
        holder = null;
        surfaceReady = false;
        if (v == null) return;
        View parent = (View) v.getParent();
        if (parent instanceof ViewGroup) {
            ((ViewGroup) parent).removeView(v);
        }
    }

    private static boolean fail(String why) {
        failure = why;
        state = ERROR;
        return false;
    }
}
