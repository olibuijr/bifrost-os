package is.olibuijr.midget;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.SharedPreferences;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.graphics.Typeface;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.text.InputType;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.widget.Button;
import android.widget.EditText;
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Remote control for Midget through the titan relay (tools/midget-relay.py). */
public class MainActivity extends Activity {
    private static final String PREFS = "midget";
    private static final String DEFAULT_URL = "http://192.168.1.12:8099";
    private static final int SLOP_PX = 24;

    private final ExecutorService io = Executors.newSingleThreadExecutor();
    private final Handler ui = new Handler(Looper.getMainLooper());
    private SharedPreferences prefs;

    private TextView status;
    private ImageView screen;
    private Bitmap shot;
    private float downX, downY;
    private long downAt;

    private static class Resp {
        int code;
        byte[] body;
        String text() { return new String(body, java.nio.charset.StandardCharsets.UTF_8); }
    }

    /* ---------- HTTP ---------- */

    private Resp request(String method, String path, String json) throws IOException {
        String base = prefs.getString("url", DEFAULT_URL);
        String token = prefs.getString("token", "");
        HttpURLConnection c = (HttpURLConnection) new URL(base + path).openConnection();
        try {
            c.setRequestMethod(method);
            c.setConnectTimeout(5000);
            c.setReadTimeout(120000);
            c.setRequestProperty("Authorization", "Bearer " + token);
            if (json != null) {
                c.setDoOutput(true);
                c.setRequestProperty("Content-Type", "application/json");
                try (OutputStream out = c.getOutputStream()) {
                    out.write(json.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                }
            }
            Resp r = new Resp();
            r.code = c.getResponseCode();
            InputStream in = r.code < 400 ? c.getInputStream() : c.getErrorStream();
            ByteArrayOutputStream buf = new ByteArrayOutputStream();
            if (in != null) {
                byte[] chunk = new byte[16384];
                int n;
                while ((n = in.read(chunk)) > 0) buf.write(chunk, 0, n);
                in.close();
            }
            r.body = buf.toByteArray();
            return r;
        } finally {
            c.disconnect();
        }
    }

    private void say(final String text) {
        ui.post(() -> status.setText(text));
    }

    /* ---------- actions ---------- */

    private void refresh() {
        io.execute(() -> {
            try {
                Resp s = request("GET", "/api/status", null);
                String state = s.code == 200 ? new JSONObject(s.text()).optString("state", "?") : "error " + s.code;
                say("Midget: " + state);
                Resp p = request("GET", "/api/screen", null);
                if (p.code == 200) {
                    Bitmap bmp = BitmapFactory.decodeByteArray(p.body, 0, p.body.length);
                    ui.post(() -> { shot = bmp; screen.setImageBitmap(bmp); });
                } else {
                    say("Midget " + state + " (no screen: " + p.text() + ")");
                }
            } catch (Exception e) {
                say("Relay unreachable: " + e.getMessage());
            }
        });
    }

    private void act(final String path, final String json) {
        io.execute(() -> {
            try {
                Resp r = request("POST", path, json);
                String msg = r.text();
                try { msg = new JSONObject(msg).optString("output", msg); } catch (Exception ignored) { }
                say((r.code == 200 ? "OK " : "Failed (" + r.code + "): ") + (msg.isEmpty() ? path : msg));
                ui.postDelayed(this::refresh, 900);
            } catch (Exception e) {
                say("Relay unreachable: " + e.getMessage());
            }
        });
    }

    private void key(String name) {
        act("/api/key", "{\"key\":\"" + name + "\"}");
    }

    /* ---------- screen touch ---------- */

    /** Maps a view pixel into original panel pixels; null when outside the picture. */
    private float[] toPanel(float x, float y) {
        if (shot == null) return null;
        int vw = screen.getWidth(), vh = screen.getHeight();
        int bw = shot.getWidth(), bh = shot.getHeight();
        float scale = Math.min((float) vw / bw, (float) vh / bh);
        float offX = (vw - bw * scale) / 2f, offY = (vh - bh * scale) / 2f;
        float px = (x - offX) / scale, py = (y - offY) / scale;
        if (px < 0 || py < 0 || px >= bw || py >= bh) return null;
        return new float[] {px, py};
    }

    private void onScreenTouch(MotionEvent e) {
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                downX = e.getX(); downY = e.getY(); downAt = e.getEventTime();
                break;
            case MotionEvent.ACTION_UP: {
                float[] a = toPanel(downX, downY), b = toPanel(e.getX(), e.getY());
                if (a == null || b == null) break;
                if (Math.hypot(e.getX() - downX, e.getY() - downY) < SLOP_PX) {
                    act("/api/tap", String.format(java.util.Locale.ROOT, "{\"x\":%.1f,\"y\":%.1f}", a[0], a[1]));
                } else {
                    long ms = Math.max(50, Math.min(5000, e.getEventTime() - downAt));
                    act("/api/swipe", String.format(java.util.Locale.ROOT,
                            "{\"x0\":%.1f,\"y0\":%.1f,\"x1\":%.1f,\"y1\":%.1f,\"ms\":%d}", a[0], a[1], b[0], b[1], ms));
                }
                break;
            }
        }
    }

    /* ---------- layout ---------- */

    private Button button(String label, View.OnClickListener l) {
        Button b = new Button(this);
        b.setText(label);
        b.setAllCaps(false);
        b.setOnClickListener(l);
        return b;
    }

    private LinearLayout row() {
        LinearLayout r = new LinearLayout(this);
        r.setOrientation(LinearLayout.HORIZONTAL);
        r.setGravity(Gravity.CENTER_VERTICAL);
        return r;
    }

    private void addButtons(LinearLayout row, String[] labels, View.OnClickListener[] ls) {
        for (int i = 0; i < labels.length; i++) row.addView(button(labels[i], ls[i]),
                new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
    }

    @Override
    protected void onCreate(Bundle saved) {
        super.onCreate(saved);
        prefs = getSharedPreferences(PREFS, MODE_PRIVATE);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(24, 24, 24, 24);
        root.setBackgroundColor(Color.rgb(20, 20, 22));

        status = new TextView(this);
        status.setTextColor(Color.WHITE);
        status.setTextSize(16);
        status.setTypeface(Typeface.DEFAULT_BOLD);
        status.setText("Midget: connecting");
        root.addView(status);

        LinearLayout top = row();
        addButtons(top, new String[] {"Refresh", "Settings"}, new View.OnClickListener[] {
                v -> refresh(), v -> settings()});
        root.addView(top);

        screen = new ImageView(this);
        screen.setBackgroundColor(Color.BLACK);
        screen.setScaleType(ImageView.ScaleType.FIT_CENTER);
        screen.setOnTouchListener((v, e) -> { onScreenTouch(e); return true; });
        root.addView(screen, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 900));

        HorizontalScrollView keysScroll = new HorizontalScrollView(this);
        LinearLayout keys = row();
        String[] names = {"Esc", "Enter", "Tab", "Space", "Bksp", "Up", "Down", "Left", "Right"};
        String[] codes = {"esc", "enter", "tab", "space", "backspace", "up", "down", "left", "right"};
        for (int i = 0; i < names.length; i++) {
            final String code = codes[i];
            Button b = button(names[i], v -> key(code));
            keys.addView(b);
        }
        keysScroll.addView(keys);
        root.addView(keysScroll);

        LinearLayout power = row();
        addButtons(power, new String[] {"Wake", "Netboot", "Disk"}, new View.OnClickListener[] {
                v -> act("/api/wake", "{}"),
                v -> confirm("Reboot Midget to netboot?", "{\"mode\":\"netboot\"}"),
                v -> confirm("Reboot Midget from disk?", "{\"mode\":\"disk\"}")});
        root.addView(power);

        LinearLayout launch = row();
        EditText appId = new EditText(this);
        appId.setHint("app id, e.g. is.olibuijr.files.desktop");
        appId.setSingleLine(true);
        appId.setInputType(InputType.TYPE_CLASS_TEXT);
        launch.addView(appId, new LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f));
        launch.addView(button("Open", v -> {
            String id = appId.getText().toString().trim();
            if (!id.isEmpty()) act("/api/open-app", "{\"id\":\"" + id.replace("\"", "") + "\"}");
        }));
        root.addView(launch);

        ScrollView scroll = new ScrollView(this);
        scroll.addView(root);
        setContentView(scroll);

        if (prefs.getString("token", "").isEmpty()) settings();
        else refresh();
    }

    private void confirm(String message, String json) {
        new AlertDialog.Builder(this)
                .setMessage(message)
                .setPositiveButton("Reboot", (d, w) -> act("/api/reboot", json))
                .setNegativeButton("Cancel", null)
                .show();
    }

    private void settings() {
        LinearLayout form = new LinearLayout(this);
        form.setOrientation(LinearLayout.VERTICAL);
        form.setPadding(48, 16, 48, 0);
        EditText url = new EditText(this);
        url.setSingleLine(true);
        url.setHint("http://192.168.1.12:8099");
        url.setText(prefs.getString("url", DEFAULT_URL));
        EditText token = new EditText(this);
        token.setSingleLine(true);
        token.setHint("relay token (out/lab/relay-token on titan)");
        token.setText(prefs.getString("token", ""));
        form.addView(url);
        form.addView(token);
        new AlertDialog.Builder(this)
                .setTitle("Relay")
                .setView(form)
                .setPositiveButton("Save", (d, w) -> {
                    prefs.edit()
                            .putString("url", url.getText().toString().trim())
                            .putString("token", token.getText().toString().trim())
                            .apply();
                    refresh();
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    @Override
    protected void onDestroy() {
        io.shutdownNow();
        super.onDestroy();
    }
}
