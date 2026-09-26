package com.agentmobile.bridge

import android.accessibilityservice.AccessibilityService
import android.content.Intent
import android.view.accessibility.AccessibilityEvent

/**
 * 桥接无障碍服务：承载点击/滑动手势、UI 树读取、截图与全局动作。
 * 服务实例由系统绑定后写入 [instance]，插件命令经此访问。
 */
class BridgeAccessibilityService : AccessibilityService() {

    override fun onServiceConnected() {
        instance = this
    }

    override fun onUnbind(intent: Intent?): Boolean {
        if (instance === this) {
            instance = null
        }
        return super.onUnbind(intent)
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // 桥接为按需命令模型，不消费事件流
    }

    override fun onInterrupt() {}

    companion object {
        @Volatile
        var instance: BridgeAccessibilityService? = null
            private set
    }
}
