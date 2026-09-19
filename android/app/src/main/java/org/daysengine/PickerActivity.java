package org.daysengine;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;
import android.view.View;
import android.widget.TextView;

/**
 * Where the player says which install to play.
 *
 * <p>This engine bundles no game data: it plays the player's own retail
 * install, and on a desktop it finds it by sitting in the same directory. A
 * phone has no such directory, so the one thing this app has to ask is which
 * folder the game is in — and it asks with Android's own folder picker rather
 * than a browser of our own, so the answer can be anywhere the player keeps
 * files.
 *
 * <p>Asked once. The grant is persisted, so every later launch goes straight
 * through to {@link DaysEngineActivity}.
 *
 * <p>This is the only screen in the whole app that is not the game's own art.
 * It is deliberately four words and a button: everything the player is here to
 * look at is composited from their install by the engine.
 */
public class PickerActivity extends Activity {

    private static final int PICK_TREE = 1;

    private TextView message;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        if (Saf.attach(this) && Saf.looksLikeAnInstall()) {
            play();
            return;
        }
        setContentView(R.layout.picker);
        message = findViewById(R.id.message);
        findViewById(R.id.choose).setOnClickListener(new View.OnClickListener() {
            @Override
            public void onClick(View view) {
                choose();
            }
        });
    }

    private void choose() {
        Intent intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION
                | Intent.FLAG_GRANT_WRITE_URI_PERMISSION
                | Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION);
        startActivityForResult(intent, PICK_TREE);
    }

    @Override
    protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_TREE) {
            return;
        }
        Uri picked = result == RESULT_OK && data != null ? data.getData() : null;
        if (picked == null) {
            return;
        }
        if (!Saf.remember(this, picked)) {
            say(R.string.not_a_folder);
            return;
        }
        // The same test the desktop makes before mounting anything. Answering
        // it here means a wrong folder is a sentence and another go, rather
        // than a mount that fails somewhere the player cannot see.
        if (!Saf.looksLikeAnInstall()) {
            Saf.forget(this);
            say(R.string.no_packs);
            return;
        }
        play();
    }

    private void say(int text) {
        if (message != null) {
            message.setText(text);
        }
    }

    private void play() {
        startActivity(new Intent(this, DaysEngineActivity.class));
        finish();
    }
}
