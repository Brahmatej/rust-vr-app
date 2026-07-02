package com.vrapp.core;

import android.content.Context;
import android.util.Log;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import androidx.core.view.InputDeviceCompat;

/* JADX INFO: loaded from: classes.dex */
public class GamepadOverlay extends View {
    private static final String TAG = "VRAppJava";
    private final MainActivity activity;
    private float lastHatX;
    private float lastHatY;

    private static boolean isFromGamepad(int i) {
        return (i & 1025) == 1025 || (i & InputDeviceCompat.SOURCE_JOYSTICK) == 16777232 || (i & InputDeviceCompat.SOURCE_DPAD) == 513;
    }

    public GamepadOverlay(Context context, MainActivity mainActivity) {
        super(context);
        this.lastHatX = 0.0f;
        this.lastHatY = 0.0f;
        this.activity = mainActivity;
        setFocusable(true);
        setFocusableInTouchMode(true);
    }

    @Override // android.view.View, android.view.KeyEvent.Callback
    public boolean onKeyDown(int i, KeyEvent keyEvent) {
        if (isFromGamepad(keyEvent.getSource())) {
            Log.i(TAG, "Overlay KEY DOWN code=" + i);
            this.activity.onGamepadButton(i, true);
            return true;
        }
        return super.onKeyDown(i, keyEvent);
    }

    @Override // android.view.View, android.view.KeyEvent.Callback
    public boolean onKeyUp(int i, KeyEvent keyEvent) {
        if (isFromGamepad(keyEvent.getSource())) {
            Log.i(TAG, "Overlay KEY UP code=" + i);
            this.activity.onGamepadButton(i, false);
            return true;
        }
        return super.onKeyUp(i, keyEvent);
    }

    @Override // android.view.View
    public boolean onGenericMotionEvent(MotionEvent motionEvent) {
        if (isFromGamepad(motionEvent.getSource()) && motionEvent.getAction() == 2) {
            float axisValue = motionEvent.getAxisValue(0);
            float axisValue2 = motionEvent.getAxisValue(1);
            float axisValue3 = motionEvent.getAxisValue(11);
            float axisValue4 = motionEvent.getAxisValue(14);
            float axisValue5 = motionEvent.getAxisValue(17);
            float axisValue6 = motionEvent.getAxisValue(18);
            if (axisValue5 == 0.0f) {
                axisValue5 = motionEvent.getAxisValue(23);
            }
            if (axisValue6 == 0.0f) {
                axisValue6 = motionEvent.getAxisValue(22);
            }
            this.activity.onGamepadAxis(axisValue, axisValue2, axisValue3, axisValue4, axisValue5, axisValue6);
            float axisValue7 = motionEvent.getAxisValue(15);
            float axisValue8 = motionEvent.getAxisValue(16);
            if (axisValue7 != this.lastHatX || axisValue8 != this.lastHatY) {
                Log.i(TAG, "Overlay HAT x=" + axisValue7 + " y=" + axisValue8);
                this.activity.onDpadAxis(axisValue7, axisValue8);
                this.lastHatX = axisValue7;
                this.lastHatY = axisValue8;
            }
            return true;
        }
        return super.onGenericMotionEvent(motionEvent);
    }
}
