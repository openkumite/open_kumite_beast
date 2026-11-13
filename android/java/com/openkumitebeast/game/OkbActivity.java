// The Android activity of the engine (OpenKumiteBeast; engine addition, docs/android.md): a NativeActivity whose native
// side (libopen_kumite_beast.so, android_main) runs the game, its setup screen included. This class does what needs the
// framework: the system's folder picker and the picked tree's files for the setup screen (listTree, openDocument),
// the input hand-over (every touch, key and joystick event to the native side through nativeInput), immersive full
// screen, keep-screen-on, the cutout's safe insets, the audio focus, a restart and the "no GPU" dialog.
package com.openkumitebeast.game;

import android.app.Activity;
import android.app.AlertDialog;
import android.app.Dialog;
import android.app.NativeActivity;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.content.res.Configuration;
import android.database.Cursor;
import android.hardware.input.InputManager;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.view.DisplayCutout;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

import java.util.ArrayList;
import java.util.List;

public class OkbActivity extends NativeActivity {
    static {
        // the same library NativeActivity opens: loaded here too so the JVM finds nativeInput in it
        System.loadLibrary("open_kumite_beast");
    }

    /** kind 0 touch (a = phase 0 down / 1 move / 2 up / 3 cancel, b = pointer id, x, y); 1 key (a = key code,
     *  b = 1 down | 2 from a gamepad); 2 joystick axes (x, y, z, w, u, v = stick x, y, hat x, y, triggers l, r);
     *  3 safe insets (x, y, z, w = left, top, right, bottom); 4 an input device removed. `dev`: the event's
     *  InputDevice id (kinds 1, 2 and 4; a gamepad's decides its player) */
    static native void nativeInput(int kind, int a, int b, int dev, float x, float y, float z, float w, float u, float v);

    /** The folder picker closed: the picked tree's URI, or null (cancelled). */
    static native void nativePicked(String uri);

    /** The launch intent's debug hooks (its `okb_env` extra: `KEY=value;…`, as the desktop's OKB_* variables). */
    static native void nativeEnv(String vars);

    private static final int PICK = 1;
    /** the picked tree's limits (the export has a few levels and about two thousand files): deeper or larger is
     *  not an export, and a cyclic or huge listing from a provider ends in a message, not a crash */
    private static final int MAX_DEPTH = 16, MAX_ENTRIES = 100_000;

    private final Handler ui = new Handler(Looper.getMainLooper());
    /** gamepads going away: their player's slot freed for the next one */
    private final InputManager.InputDeviceListener devices = new InputManager.InputDeviceListener() {
        @Override
        public void onInputDeviceAdded(int id) {
        }

        @Override
        public void onInputDeviceRemoved(int id) {
            nativeInput(4, 0, 0, id, 0, 0, 0, 0, 0, 0);
        }

        @Override
        public void onInputDeviceChanged(int id) {
        }
    };
    private AudioFocusRequest focus;

    @Override
    protected void onCreate(Bundle saved) {
        // before the native side starts (super.onCreate starts it); only in a debuggable build (the launcher activity
        // is exported: another app could otherwise start it with scripts and paths)
        boolean debuggable = (getApplicationInfo().flags & ApplicationInfo.FLAG_DEBUGGABLE) != 0;
        String env = null;
        if (debuggable && getIntent() != null) {
            env = getIntent().getStringExtra("okb_env");
            if (env == null) {
                env = getIntent().getStringExtra("br2_env");
            }
        }
        if (env != null) {
            nativeEnv(env);
        }
        super.onCreate(saved);
        // the input goes to the view hierarchy (and from there to nativeInput) instead of NativeActivity's queue
        getWindow().takeInputQueue(null);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        // the picture under the cutout too (in landscape the default mode letterboxes its side); the touch controls
        // stay inside the safe insets
        cutout(getWindow());
        View root = getWindow().getDecorView();
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            sendInsets(insets);
            return v.onApplyWindowInsets(insets);
        });
        immersive(getWindow());
        InputManager im = (InputManager) getSystemService(Context.INPUT_SERVICE);
        if (im != null) {
            im.registerInputDeviceListener(devices, ui);
        }
    }

    /** A change the manifest's configChanges lists (every public one: rotation, size, dark mode, font scale and
     *  weight, density, keyboard, language …): the activity stays, NativeActivity tells the native side (winit then
     *  resizes the surface if needed); the bars hidden and the insets sent again. */
    @Override
    public void onConfigurationChanged(Configuration c) {
        super.onConfigurationChanged(c);
        immersive(getWindow());
        getWindow().getDecorView().requestApplyInsets();
    }

    /** NativeActivity's destroy waits for android_main to return, which winit's loop never does on its own, and a
     *  winit event loop is made once per process: the process ends here. An activity the system recreates (a
     *  change configChanges cannot list, e.g. a theme overlay's assets) is started again in a fresh process; one
     *  the system destroys in the background (low memory) is not brought back to the front. The game saved its card
     *  and preferences when they changed. */
    @Override
    protected void onDestroy() {
        if (isChangingConfigurations()) {
            Intent i = getPackageManager().getLaunchIntentForPackage(getPackageName());
            if (i != null) {
                i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
                startActivity(i);
            }
        }
        Runtime.getRuntime().exit(0);
    }

    @Override
    public void onWindowFocusChanged(boolean has) {
        super.onWindowFocusChanged(has);
        if (has) {
            immersive(getWindow());
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        immersive(getWindow());
        AudioManager am = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        if (am != null) {
            focus = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(new AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_GAME)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                            .build())
                    .build();
            am.requestAudioFocus(focus);
        }
    }

    @Override
    protected void onPause() {
        AudioManager am = (AudioManager) getSystemService(Context.AUDIO_SERVICE);
        if (am != null && focus != null) {
            am.abandonAudioFocusRequest(focus);
        }
        super.onPause();
    }

    /** Full screen at all times: the status and navigation bars hidden, a swipe from the edge shows them over the
     *  picture for a while (they hide again by themselves). For the activity's window (on create, resume, focus
     *  back, a configuration change) and for each dialog's (`show`): a dialog's window takes the focus, and on
     *  API 26–29 the focused window's flags decide the bars, on API 30+ its requested visibility does. */
    @SuppressWarnings("deprecation")
    private static void immersive(Window w) {
        View d = w.getDecorView();
        if (Build.VERSION.SDK_INT >= 30) {
            w.setDecorFitsSystemWindows(false);
            WindowInsetsController c = d.getWindowInsetsController();
            if (c != null) {
                c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                c.hide(WindowInsets.Type.systemBars());
            }
        } else {
            d.setSystemUiVisibility(View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN);
        }
    }

    /** The window drawn into the display cutout's area as well: ALWAYS on API 30+ (every edge), SHORT_EDGES on
     *  28–29 (the cutouts of that era sit on a short edge, the sides in landscape); API 26–27 have no cutout API. */
    private static void cutout(Window w) {
        if (Build.VERSION.SDK_INT >= 28) {
            WindowManager.LayoutParams lp = w.getAttributes();
            lp.layoutInDisplayCutoutMode = Build.VERSION.SDK_INT >= 30
                    ? WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
                    : WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
            w.setAttributes(lp);
        }
    }

    /** A dialog shown without bringing the system bars back: its window starts unfocusable (taking the focus is
     *  what shows them), is made full screen like the activity's, then takes input. */
    private AlertDialog show(AlertDialog.Builder b) {
        return show(b.create());
    }

    private <D extends Dialog> D show(D d) {
        Window w = d.getWindow();
        if (w != null) {
            w.addFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE);
            cutout(w);
        }
        d.show();
        if (w != null) {
            immersive(w);
            w.clearFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE);
        }
        return d;
    }

    private void sendInsets(WindowInsets insets) {
        DisplayCutout c = insets.getDisplayCutout();
        if (c != null) {
            nativeInput(3, 0, 0, 0, c.getSafeInsetLeft(), c.getSafeInsetTop(), c.getSafeInsetRight(), c.getSafeInsetBottom(), 0, 0);
        } else {
            nativeInput(3, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        }
    }

    // ------------------------------------------------------------------ input

    @Override
    public boolean dispatchTouchEvent(MotionEvent e) {
        int action = e.getActionMasked();
        switch (action) {
            case MotionEvent.ACTION_DOWN:
            case MotionEvent.ACTION_POINTER_DOWN:
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_POINTER_UP: {
                int i = e.getActionIndex();
                boolean down = action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN;
                nativeInput(0, down ? 0 : 2, e.getPointerId(i), 0, e.getX(i), e.getY(i), 0, 0, 0, 0);
                break;
            }
            case MotionEvent.ACTION_MOVE:
                for (int i = 0; i < e.getPointerCount(); i++) {
                    nativeInput(0, 1, e.getPointerId(i), 0, e.getX(i), e.getY(i), 0, 0, 0, 0);
                }
                break;
            case MotionEvent.ACTION_CANCEL:
                for (int i = 0; i < e.getPointerCount(); i++) {
                    nativeInput(0, 3, e.getPointerId(i), 0, e.getX(i), e.getY(i), 0, 0, 0, 0);
                }
                break;
            default:
                break;
        }
        return true;
    }

    @Override
    public boolean dispatchGenericMotionEvent(MotionEvent e) {
        if ((e.getSource() & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK && e.getAction() == MotionEvent.ACTION_MOVE) {
            float lt = Math.max(e.getAxisValue(MotionEvent.AXIS_LTRIGGER), e.getAxisValue(MotionEvent.AXIS_BRAKE));
            float rt = Math.max(e.getAxisValue(MotionEvent.AXIS_RTRIGGER), e.getAxisValue(MotionEvent.AXIS_GAS));
            nativeInput(2, 0, 0, e.getDeviceId(), e.getAxisValue(MotionEvent.AXIS_X), e.getAxisValue(MotionEvent.AXIS_Y),
                    e.getAxisValue(MotionEvent.AXIS_HAT_X), e.getAxisValue(MotionEvent.AXIS_HAT_Y), lt, rt);
            return true;
        }
        return super.dispatchGenericMotionEvent(e);
    }

    @Override
    public boolean dispatchKeyEvent(KeyEvent e) {
        int code = e.getKeyCode();
        // the volume keys and the like stay the system's
        if (code == KeyEvent.KEYCODE_VOLUME_UP || code == KeyEvent.KEYCODE_VOLUME_DOWN || code == KeyEvent.KEYCODE_VOLUME_MUTE
                || code == KeyEvent.KEYCODE_POWER || code == KeyEvent.KEYCODE_HOME || code == KeyEvent.KEYCODE_APP_SWITCH) {
            return super.dispatchKeyEvent(e);
        }
        int a = e.getAction();
        if ((a == KeyEvent.ACTION_DOWN && e.getRepeatCount() == 0) || a == KeyEvent.ACTION_UP) {
            // from a gamepad: by the event's source, or a device with sticks (a pad's d-pad may come as a plain DPAD
            // key; a keyboard may list gamepad buttons but has no sticks)
            int src = e.getSource();
            InputDevice d = e.getDevice();
            boolean pad = (src & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                    || (src & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
                    || (d != null && d.supportsSource(InputDevice.SOURCE_JOYSTICK));
            nativeInput(1, code, (a == KeyEvent.ACTION_DOWN ? 1 : 0) | (pad ? 2 : 0), e.getDeviceId(), 0, 0, 0, 0, 0, 0);
        }
        return true;
    }

    // ------------------------------------------------------------------ the setup screen's calls

    /** The game cannot start (no GPU: nothing can be drawn): the reason in a dialog, then a clean end through the
     *  activity (`finish`, `onDestroy`). Called by the native side on its thread; it does nothing more meanwhile. */
    public void showFatal(String msg) {
        ui.post(() -> show(new AlertDialog.Builder(this)
                .setTitle("The game could not start")
                .setMessage(msg)
                .setCancelable(false)
                .setNegativeButton("Quit", (d, w) -> finish())));
    }

    /** QUIT on the setup screen: the clean end through the activity. */
    public void quit() {
        ui.post(this::finish);
    }

    /** New game data installed while the game ran on the old: the app starts again on it. */
    public void restartApp() {
        ui.post(() -> {
            Intent i = getPackageManager().getLaunchIntentForPackage(getPackageName());
            if (i != null) {
                i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_CLEAR_TASK);
                startActivity(i);
            }
            Runtime.getRuntime().exit(0);
        });
    }

    /** The free space of the app's storage. */
    public long freeSpace() {
        return getFilesDir().getUsableSpace();
    }

    /** The system's document picker for a disc image (starting in Download); the answer goes to `nativePicked`. */
    public void pickFile() {
        ui.post(() -> {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT);
            i.addCategory(Intent.CATEGORY_OPENABLE);
            i.setType("*/*");
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            i.putExtra(DocumentsContract.EXTRA_INITIAL_URI,
                    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Download"));
            try {
                startActivityForResult(i, PICK);
            } catch (Exception ex) {
                android.util.Log.w("okb", "no document picker: " + ex);
                nativePicked(null);
            }
        });
    }

    /** A picked document opened for reading (called on a native thread): a file descriptor the native side owns
     *  from here, or -1. */
    public int openUri(String uri) {
        try {
            ParcelFileDescriptor p = getContentResolver().openFileDescriptor(Uri.parse(uri), "r");
            return p == null ? -1 : p.detachFd();
        } catch (Throwable ex) {
            android.util.Log.w("okb", "open " + uri + ": " + ex);
            return -1;
        }
    }

    /** The system's folder picker (starting in Download); the answer goes to `nativePicked`. */
    public void pickFolder() {
        ui.post(() -> {
            Intent i = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            i.putExtra(DocumentsContract.EXTRA_INITIAL_URI,
                    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", "primary:Download"));
            try {
                startActivityForResult(i, PICK);
            } catch (Exception ex) {
                android.util.Log.w("okb", "no folder picker: " + ex);
                nativePicked(null);
            }
        });
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK) {
            return;
        }
        boolean ok = result == Activity.RESULT_OK && data != null && data.getData() != null;
        nativePicked(ok ? data.getData().toString() : null);
    }

    private static final String[] COLUMNS = {DocumentsContract.Document.COLUMN_DOCUMENT_ID,
            DocumentsContract.Document.COLUMN_DISPLAY_NAME, DocumentsContract.Document.COLUMN_MIME_TYPE,
            DocumentsContract.Document.COLUMN_SIZE};

    /** The children of a document: (name, document id, is a folder, size; an unknown size is -1). */
    private List<Object[]> children(Uri tree, String doc) throws Exception {
        List<Object[]> out = new ArrayList<>();
        Uri uri = DocumentsContract.buildChildDocumentsUriUsingTree(tree, doc);
        try (Cursor c = getContentResolver().query(uri, COLUMNS, null, null, null)) {
            while (c != null && c.moveToNext()) {
                if (out.size() >= MAX_ENTRIES) {
                    throw new Exception("This folder has too many files to be game data.");
                }
                boolean dir = DocumentsContract.Document.MIME_TYPE_DIR.equals(c.getString(2));
                out.add(new Object[] {c.getString(1), c.getString(0), dir, c.isNull(3) ? -1L : Math.max(0L, c.getLong(3))});
            }
        }
        return out;
    }

    /** Every file under `doc` as "path\tdocument id\tsize\n" lines; hidden files and the web build (`play/`) left.
     *  A name the provider gives with a path separator, a tab, a newline or a NUL in it is refused: it could reach
     *  outside the copy's folder or break the listing. */
    private void walk(Uri tree, String doc, String rel, int depth, int[] seen, StringBuilder out) throws Exception {
        if (depth > MAX_DEPTH) {
            throw new Exception("This folder is nested too deeply to be game data.");
        }
        for (Object[] ch : children(tree, doc)) {
            String name = (String) ch[0];
            if (name == null || name.startsWith(".") || (rel.isEmpty() && name.equals("play"))) {
                continue;
            }
            if (name.indexOf('/') >= 0 || name.indexOf('\\') >= 0 || name.indexOf('\0') >= 0 || name.indexOf('\t') >= 0
                    || name.indexOf('\n') >= 0) {
                throw new Exception("The folder has a file with an unusable name: " + rel + name.replace('\0', '?'));
            }
            // files and folders both count
            if (++seen[0] > MAX_ENTRIES) {
                throw new Exception("This folder has too many files to be game data.");
            }
            if ((Boolean) ch[2]) {
                walk(tree, (String) ch[1], rel + name + "/", depth + 1, seen, out);
            } else {
                out.append(rel).append(name).append('\t').append((String) ch[1]).append('\t').append((Long) ch[3]).append('\n');
            }
        }
    }

    /** A picked tree's files for the setup screen (called on a native thread): the listing, or "!" and why not. */
    public String listTree(String uri) {
        try {
            Uri tree = Uri.parse(uri);
            StringBuilder out = new StringBuilder();
            walk(tree, DocumentsContract.getTreeDocumentId(tree), "", 0, new int[1], out);
            return out.toString();
        } catch (Throwable ex) {
            return "!" + (ex.getMessage() != null ? ex.getMessage() : ex.toString());
        }
    }

    /** A document of a picked tree opened for reading (called on a native thread): a file descriptor the native side
     *  owns from here, or -1. */
    public int openDocument(String uri, String doc) {
        try {
            Uri tree = Uri.parse(uri);
            ParcelFileDescriptor p = getContentResolver().openFileDescriptor(DocumentsContract.buildDocumentUriUsingTree(tree, doc), "r");
            return p == null ? -1 : p.detachFd();
        } catch (Throwable ex) {
            android.util.Log.w("okb", "open " + doc + ": " + ex);
            return -1;
        }
    }
}
