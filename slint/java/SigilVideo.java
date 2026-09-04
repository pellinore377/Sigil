// Video, played by the phone.
//
// The app draws everything through one surface of its own, and has no
// decoder for video on Android. The phone has both a decoder and a view that
// wraps it, so a playing video is that view laid over the app's surface at
// the rectangle the viewer would have drawn it in, and taken away again when
// the viewer closes. Everything here is static and called from the engine's
// threads; every touch of a view hops to the main thread, and the questions
// (position, duration, playing) read fields the view updates.
//
// Loaded at runtime from an embedded dex (see build.rs and platform.rs),
// alongside SigilFilePicker, so nothing here can be named in the manifest —
// which is fine: a view needs no manifest entry.

import android.app.Activity;
import android.media.MediaPlayer;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.VideoView;

public final class SigilVideo {
    private static VideoView view;
    private static Activity host;
    private static volatile boolean ready;
    private static volatile boolean ended;
    private static volatile String failure;

    private SigilVideo() {}

    /// Lay the view over the activity at (x, y, w, h) in physical pixels and
    /// start the file. A second call replaces the first.
    public static void show(final Activity activity, final String path,
                            final int x, final int y, final int w, final int h) {
        host = activity;
        ready = false;
        ended = false;
        failure = null;
        activity.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                dropView();
                VideoView v = new VideoView(activity);
                // Above the app's own surface, which is a SurfaceView too.
                v.setZOrderOnTop(true);
                FrameLayout.LayoutParams lp = new FrameLayout.LayoutParams(w, h);
                lp.gravity = Gravity.TOP | Gravity.START;
                lp.leftMargin = x;
                lp.topMargin = y;
                v.setOnPreparedListener(new MediaPlayer.OnPreparedListener() {
                    @Override
                    public void onPrepared(MediaPlayer mp) {
                        ready = true;
                        mp.setLooping(false);
                        v.start();
                    }
                });
                v.setOnCompletionListener(new MediaPlayer.OnCompletionListener() {
                    @Override
                    public void onCompletion(MediaPlayer mp) {
                        ended = true;
                    }
                });
                v.setOnErrorListener(new MediaPlayer.OnErrorListener() {
                    @Override
                    public boolean onError(MediaPlayer mp, int what, int extra) {
                        failure = "MediaPlayer error " + what + "/" + extra;
                        return true;
                    }
                });
                activity.addContentView(v, lp);
                v.setVideoPath(path);
                view = v;
            }
        });
    }

    /// The viewer moved or resized the picture: follow it.
    public static void move(final int x, final int y, final int w, final int h) {
        final Activity a = host;
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                VideoView v = view;
                if (v == null) return;
                ViewGroup.LayoutParams raw = v.getLayoutParams();
                if (!(raw instanceof FrameLayout.LayoutParams)) return;
                FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) raw;
                lp.width = w;
                lp.height = h;
                lp.leftMargin = x;
                lp.topMargin = y;
                v.setLayoutParams(lp);
            }
        });
    }

    public static void pause() {
        final Activity a = host;
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                VideoView v = view;
                if (v != null && v.isPlaying()) v.pause();
            }
        });
    }

    public static void resume() {
        final Activity a = host;
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                VideoView v = view;
                if (v != null) {
                    ended = false;
                    v.start();
                }
            }
        });
    }

    public static void seekTo(final int ms) {
        final Activity a = host;
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                VideoView v = view;
                if (v != null) {
                    ended = false;
                    v.seekTo(ms);
                }
            }
        });
    }

    /// Milliseconds into the clip; 0 before it is ready.
    public static int position() {
        VideoView v = view;
        return (v != null && ready) ? v.getCurrentPosition() : 0;
    }

    /// The clip's length in milliseconds; 0 before it is known.
    public static int duration() {
        VideoView v = view;
        return (v != null && ready) ? Math.max(0, v.getDuration()) : 0;
    }

    public static boolean isPlaying() {
        VideoView v = view;
        return v != null && ready && v.isPlaying() && !ended;
    }

    public static boolean hasEnded() {
        return ended;
    }

    /// The last error, or null.
    public static String failure() {
        return failure;
    }

    /// Stop and take the view away.
    public static void hide() {
        final Activity a = host;
        if (a == null) return;
        a.runOnUiThread(new Runnable() {
            @Override
            public void run() {
                dropView();
            }
        });
    }

    private static void dropView() {
        VideoView v = view;
        view = null;
        ready = false;
        if (v == null) return;
        try {
            v.stopPlayback();
        } catch (RuntimeException ignored) {
        }
        View parent = (View) v.getParent();
        if (parent instanceof ViewGroup) {
            ((ViewGroup) parent).removeView(v);
        }
    }
}
