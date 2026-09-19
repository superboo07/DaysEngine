package org.daysengine;

import android.content.ContentResolver;
import android.content.Context;
import android.content.SharedPreferences;
import android.database.Cursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract;
import android.util.Log;

import java.util.ArrayList;
import java.util.List;

/**
 * The player's chosen install, reached the only way Android allows.
 *
 * <p>There is no directory an app may open by name. What the player grants in
 * the system folder picker is a <em>tree</em>, and a file inside it is reached
 * by document id through {@link ContentResolver}. So the engine cannot use
 * {@code std::fs} there, and every one of its reads and writes arrives here
 * instead: {@code src/install/saf.rs} is the other side of these methods, and
 * it is what turns a made-up path like {@code /saf/Packs/System.GPK} into the
 * document ids below.
 *
 * <p><b>Opening is the only expensive thing that happens often, and it does
 * not.</b> {@link #open} hands back a real file descriptor, detached from its
 * {@link ParcelFileDescriptor} so that the native side owns it, and a local
 * document's descriptor is seekable. A pack is therefore opened once and then
 * read with ordinary seeks for the rest of the session, which is what makes a
 * twenty-gigabyte install workable without ever pulling it through Binder.
 *
 * <p>Nothing here is bundled game data or recovered behaviour. It is Android's
 * own API doing what the desktop's filesystem does for free.
 */
public final class Saf {
    private static final String TAG = "DaysEngine";
    private static final String PREFS = "daysengine";
    private static final String KEY_TREE = "tree";

    private static Context context;
    private static Uri tree;
    private static String rootDocument;

    private Saf() {
    }

    /**
     * Brings back a folder the player granted in an earlier session.
     *
     * <p>A persisted permission outlives the process, so the picker is asked
     * for once and never again unless the grant is revoked or the folder goes
     * away. Returns false when there is nothing to come back to, which is what
     * sends the player to {@link PickerActivity}.
     */
    static boolean attach(Context ctx) {
        context = ctx.getApplicationContext();
        if (tree != null) {
            return true;
        }
        SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String saved = prefs.getString(KEY_TREE, null);
        if (saved == null) {
            return false;
        }
        Uri candidate = Uri.parse(saved);
        if (!holdsPermission(candidate)) {
            Log.w(TAG, "the grant for " + saved + " is gone");
            return false;
        }
        return adopt(candidate);
    }

    /** Takes a freshly picked tree and keeps it for next time. */
    static boolean remember(Context ctx, Uri picked) {
        context = ctx.getApplicationContext();
        try {
            context.getContentResolver().takePersistableUriPermission(
                    picked, android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION
                            | android.content.Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
        } catch (SecurityException e) {
            Log.w(TAG, "cannot hold on to " + picked, e);
        }
        if (!adopt(picked)) {
            return false;
        }
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .edit().putString(KEY_TREE, picked.toString()).apply();
        return true;
    }

    /** Forgets the granted folder, so the picker comes up again. */
    static void forget(Context ctx) {
        tree = null;
        rootDocument = null;
        ctx.getApplicationContext().getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .edit().remove(KEY_TREE).apply();
    }

    private static boolean adopt(Uri picked) {
        try {
            tree = picked;
            rootDocument = DocumentsContract.getTreeDocumentId(picked);
            return rootDocument != null;
        } catch (RuntimeException e) {
            Log.w(TAG, "not a document tree: " + picked, e);
            tree = null;
            rootDocument = null;
            return false;
        }
    }

    private static boolean holdsPermission(Uri candidate) {
        for (android.content.UriPermission held
                : context.getContentResolver().getPersistedUriPermissions()) {
            if (held.getUri().equals(candidate) && held.isReadPermission()) {
                return true;
            }
        }
        return false;
    }

    /**
     * Whether a folder looks like a game install.
     *
     * <p>The same test the desktop makes: a directory called {@code Packs}
     * beside the executable. Checked here rather than left to the engine so
     * that picking the wrong folder is answered by the picker, which can ask
     * again, instead of by a failed mount the player cannot act on.
     */
    static boolean looksLikeAnInstall() {
        if (rootDocument == null) {
            return false;
        }
        for (String row : listRows(rootDocument)) {
            String[] parts = row.split("\t", -1);
            if (parts.length == 3 && parts[2].equals("d") && parts[0].equalsIgnoreCase("Packs")) {
                return true;
            }
        }
        return false;
    }

    private static Uri document(String documentId) {
        return DocumentsContract.buildDocumentUriUsingTree(tree, documentId);
    }

    // ---------------------------------------------------------------
    // Called from native. src/install/saf.rs names each of these by its
    // signature, so a change here is a change there.
    // ---------------------------------------------------------------

    /** The document id of the folder the player granted, or null. */
    public static String rootDocument() {
        return rootDocument;
    }

    /**
     * Every child of a directory, three strings each: name, document id, and
     * {@code "d"} or {@code "f"}.
     *
     * <p>Flat strings rather than an array of some class of ours, because
     * three {@code GetObjectArrayElement} calls and no class lookup is the
     * whole of the marshalling on the other side.
     */
    public static String[] list(String documentId) {
        List<String> rows = listRows(documentId);
        List<String> flat = new ArrayList<>(rows.size() * 3);
        for (String row : rows) {
            String[] parts = row.split("\t", -1);
            flat.add(parts[0]);
            flat.add(parts[1]);
            flat.add(parts[2]);
        }
        return flat.toArray(new String[0]);
    }

    private static List<String> listRows(String documentId) {
        List<String> out = new ArrayList<>();
        if (tree == null) {
            return out;
        }
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, documentId);
        String[] columns = {
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_MIME_TYPE,
        };
        try (Cursor cursor = context.getContentResolver()
                .query(children, columns, null, null, null)) {
            if (cursor == null) {
                return out;
            }
            while (cursor.moveToNext()) {
                String name = cursor.getString(0);
                String id = cursor.getString(1);
                String mime = cursor.getString(2);
                if (name == null || id == null) {
                    continue;
                }
                boolean isDir = DocumentsContract.Document.MIME_TYPE_DIR.equals(mime);
                out.add(name + "\t" + id + "\t" + (isDir ? "d" : "f"));
            }
        } catch (Exception e) {
            Log.w(TAG, "cannot list " + documentId, e);
        }
        return out;
    }

    /**
     * Opens a document and gives up ownership of the descriptor.
     *
     * <p>{@code detachFd} is what makes the native {@code File} the owner:
     * without it the descriptor would close when this method's
     * {@link ParcelFileDescriptor} was collected, at a moment nothing on the
     * other side can predict.
     */
    public static int open(String documentId, String mode) {
        try {
            ParcelFileDescriptor pfd = context.getContentResolver()
                    .openFileDescriptor(document(documentId), mode);
            if (pfd == null) {
                return -1;
            }
            return pfd.detachFd();
        } catch (Exception e) {
            Log.w(TAG, "cannot open " + documentId + " as " + mode + ": " + e.getMessage());
            return -1;
        }
    }

    /** Creates a child document and returns its id, or null. */
    public static String create(String parentDocumentId, String mime, String name) {
        try {
            Uri made = DocumentsContract.createDocument(
                    context.getContentResolver(), document(parentDocumentId), mime, name);
            return made == null ? null : DocumentsContract.getDocumentId(made);
        } catch (Exception e) {
            Log.w(TAG, "cannot create " + name + " in " + parentDocumentId, e);
            return null;
        }
    }

    /** Renames a document in place. */
    public static boolean rename(String documentId, String name) {
        try {
            return DocumentsContract.renameDocument(
                    context.getContentResolver(), document(documentId), name) != null;
        } catch (Exception e) {
            Log.w(TAG, "cannot rename " + documentId + " to " + name, e);
            return false;
        }
    }

    /** Deletes a document. */
    public static boolean delete(String documentId) {
        try {
            return DocumentsContract.deleteDocument(
                    context.getContentResolver(), document(documentId));
        } catch (Exception e) {
            Log.w(TAG, "cannot delete " + documentId, e);
            return false;
        }
    }
}
