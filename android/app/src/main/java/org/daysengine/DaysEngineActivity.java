package org.daysengine;

import android.content.Intent;
import android.os.Bundle;

import org.libsdl.app.SDLActivity;

/**
 * The game.
 *
 * <p>An Android app has no {@code main}. {@link SDLActivity} loads the shared
 * objects {@link #getLibraries} names, then looks up {@code SDL_main} in the
 * last of them and calls it on a thread of its own — so what runs here is
 * {@code src/android.rs}, which ends in the same loop every other platform
 * runs.
 *
 * <p>Touch needs nothing from this class. SDL reports a tap as a mouse motion
 * to where the finger landed followed by a left button press at the same
 * point, so every screen in the engine — all of which highlight what the
 * pointer is over and act on the click — works under a finger unchanged. The
 * two hints that pin that behaviour down are set in {@code src/android.rs}.
 */
public class DaysEngineActivity extends SDLActivity {

    /**
     * Loaded in dependency order, engine last.
     *
     * <p>Last matters: {@link SDLActivity#getMainSharedObject} takes the final
     * name here as the library to find {@code SDL_main} in. The av* libraries
     * would be resolved by the dynamic linker anyway, being in the same
     * directory, but naming them makes the order this app depends on visible
     * rather than inherited from the linker's search.
     */
    @Override
    protected String[] getLibraries() {
        return new String[] {
            "SDL3",
            "avutil",
            "swresample",
            "swscale",
            "avcodec",
            "avformat",
            "avfilter",
            "daysengine",
        };
    }

    /**
     * There is no command line on a phone.
     *
     * <p>SDL passes these to {@code SDL_main}, which ignores them — see the
     * note on {@code android::boot}. Returning nothing rather than leaving the
     * default keeps the two ends saying the same thing.
     */
    @Override
    protected String[] getArguments() {
        return new String[0];
    }

    /**
     * Makes sure there is a granted folder before SDL starts.
     *
     * <p>The picker is the launcher activity, so ordinarily this is reached
     * with one already attached. It is reached without one when the process
     * was killed and Android restored this activity straight from the task
     * stack, which resets every static in the app — so the grant is brought
     * back from preferences here too, and a player whose grant has been
     * revoked is sent back to the picker rather than into a mount that cannot
     * work.
     */
    @Override
    protected void onCreate(Bundle savedInstanceState) {
        boolean ready = Saf.attach(this);
        super.onCreate(savedInstanceState);
        if (!ready) {
            startActivity(new Intent(this, PickerActivity.class));
            finish();
        }
    }
}
