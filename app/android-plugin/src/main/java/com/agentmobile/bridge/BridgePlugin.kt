package com.agentmobile.bridge

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.app.Activity
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Path
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.util.Base64
import android.view.Display
import android.view.accessibility.AccessibilityNodeInfo
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream

/**
 * Tauri 插件：把无障碍能力暴露给 Rust core（run_mobile_plugin 调用）。
 * 命令名与 Rust 侧 / 桥接协议动作一一对应：
 * tap / swipe / input / key / uiTree / screenshot / apps / launch / a11yStatus / openA11ySettings。
 */
@TauriPlugin
class BridgePlugin(private val activity: Activity) : Plugin(activity) {

    private fun serviceOrReject(invoke: Invoke): BridgeAccessibilityService? {
        val svc = BridgeAccessibilityService.instance
        if (svc == null) {
            invoke.reject("无障碍服务未开启：请在系统设置中启用 Agent Mobile Bridge")
        }
        return svc
    }

    // ---------- tap / swipe（dispatchGesture） ----------

    @Command
    fun tap(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        val args = invoke.getArgs()
        val x = args.getDouble("x").toFloat()
        val y = args.getDouble("y").toFloat()
        val path = Path().apply {
            moveTo(x, y)
            lineTo(x + 1f, y + 1f)
        }
        val gesture = GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(path, 0L, TAP_DURATION_MS))
            .build()
        dispatch(invoke, svc, gesture, "点击手势")
    }

    @Command
    fun swipe(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        val args = invoke.getArgs()
        val path = Path().apply {
            moveTo(args.getDouble("x1").toFloat(), args.getDouble("y1").toFloat())
            lineTo(args.getDouble("x2").toFloat(), args.getDouble("y2").toFloat())
        }
        val duration = args.getLong("duration_ms").coerceAtLeast(1L)
        val gesture = GestureDescription.Builder()
            .addStroke(GestureDescription.StrokeDescription(path, 0L, duration))
            .build()
        dispatch(invoke, svc, gesture, "滑动手势")
    }

    private fun dispatch(
        invoke: Invoke,
        svc: BridgeAccessibilityService,
        gesture: GestureDescription,
        label: String,
    ) {
        try {
            val accepted = svc.dispatchGesture(
                gesture,
                object : AccessibilityService.GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription) {
                        invoke.resolve()
                    }

                    override fun onCancelled(gestureDescription: GestureDescription) {
                        invoke.reject("$label 被系统取消")
                    }
                },
                null,
            )
            if (!accepted) {
                invoke.reject("$label 未能分发（服务未就绪）")
            }
        } catch (e: Exception) {
            invoke.reject("$label 分发失败: ${e.message}")
        }
    }

    // ---------- uiTree（uiautomator dump 同构 XML） ----------

    @Command
    fun uiTree(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        val root = svc.rootInActiveWindow
        if (root == null) {
            invoke.reject("无法获取当前窗口根节点（rootInActiveWindow 为空）")
            return
        }
        try {
            val xml = StringBuilder()
            xml.append("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>")
            xml.append("<hierarchy rotation=\"0\">")
            serializeNode(root, xml, 0)
            xml.append("</hierarchy>")
            invoke.resolve(JSObject().put("xml", xml.toString()))
        } catch (e: Exception) {
            invoke.reject("UI 树序列化失败: ${e.message}")
        }
    }

    private fun serializeNode(node: AccessibilityNodeInfo, out: StringBuilder, depth: Int) {
        if (depth > MAX_TREE_DEPTH) {
            // 深度截断可观测：超深节点不再静默丢弃，而是在其父节点下输出占位节点，
            // 便于排查 snapshot 缺节点问题（每个被截断的子节点各产生一个占位）。
            out.append("<node")
            attr(out, "index", "0")
            attr(out, "text", "[truncated: depth>$MAX_TREE_DEPTH]")
            attr(out, "resource-id", "")
            attr(out, "class", "bridge.Truncated")
            attr(out, "package", node.packageName?.toString() ?: "")
            attr(out, "content-desc", "")
            attr(out, "bounds", "[0,0][0,0]")
            out.append("/>")
            return
        }
        val bounds = Rect()
        node.getBoundsInScreen(bounds)
        out.append("<node")
        attr(out, "index", "0")
        attr(out, "text", node.text?.toString() ?: "")
        attr(out, "resource-id", node.viewIdResourceName ?: "")
        attr(out, "class", node.className?.toString() ?: "")
        attr(out, "package", node.packageName?.toString() ?: "")
        attr(out, "content-desc", node.contentDescription?.toString() ?: "")
        attr(out, "checkable", node.isCheckable)
        attr(out, "checked", node.isChecked)
        attr(out, "clickable", node.isClickable)
        attr(out, "enabled", node.isEnabled)
        attr(out, "focusable", node.isFocusable)
        attr(out, "focused", node.isFocused)
        attr(out, "scrollable", node.isScrollable)
        attr(out, "long-clickable", node.isLongClickable)
        attr(out, "password", node.isPassword)
        attr(out, "selected", node.isSelected)
        attr(out, "bounds", "[${bounds.left},${bounds.top}][${bounds.right},${bounds.bottom}]")
        if (node.childCount == 0) {
            out.append("/>")
            return
        }
        out.append(">")
        for (i in 0 until node.childCount) {
            node.getChild(i)?.let { serializeNode(it, out, depth + 1) }
        }
        out.append("</node>")
    }

    private fun attr(out: StringBuilder, name: String, value: Boolean) {
        attr(out, name, if (value) "true" else "false")
    }

    private fun attr(out: StringBuilder, name: String, value: String) {
        out.append(' ').append(name).append("=\"")
        for (c in value) {
            when (c) {
                '<' -> out.append("&lt;")
                '>' -> out.append("&gt;")
                '&' -> out.append("&amp;")
                '"' -> out.append("&quot;")
                else -> out.append(c)
            }
        }
        out.append('"')
    }

    // ---------- screenshot（API 30+ takeScreenshot） ----------

    @Command
    fun screenshot(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) {
            invoke.reject("截图需要 Android 11（API 30）及以上")
            return
        }
        try {
            svc.takeScreenshot(
                Display.DEFAULT_DISPLAY,
                ContextCompat.getMainExecutor(activity),
                object : AccessibilityService.TakeScreenshotCallback {
                    override fun onSuccess(result: AccessibilityService.ScreenshotResult) {
                        try {
                            val hardware =
                                Bitmap.wrapHardwareBuffer(result.hardwareBuffer, result.colorSpace)
                            result.hardwareBuffer.close()
                            if (hardware == null) {
                                invoke.reject("截图失败：wrapHardwareBuffer 返回空")
                                return
                            }
                            val software = hardware.copy(Bitmap.Config.ARGB_8888, false)
                            hardware.recycle()
                            if (software == null) {
                                invoke.reject("截图失败：位图转换为空")
                                return
                            }
                            val stream = ByteArrayOutputStream()
                            software.compress(Bitmap.CompressFormat.PNG, 100, stream)
                            software.recycle()
                            invoke.resolve(
                                JSObject().put(
                                    "png_base64",
                                    Base64.encodeToString(stream.toByteArray(), Base64.NO_WRAP),
                                ),
                            )
                        } catch (e: Exception) {
                            invoke.reject("截图编码失败: ${e.message}")
                        }
                    }

                    override fun onFailure(errorCode: Int) {
                        invoke.reject("截图失败，系统错误码 $errorCode")
                    }
                },
            )
        } catch (e: Exception) {
            invoke.reject("截图请求失败: ${e.message}")
        }
    }

    // ---------- input（对当前焦点输入框 ACTION_SET_TEXT） ----------

    @Command
    fun input(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        val text = invoke.getArgs().getString("text")
        val root = svc.rootInActiveWindow
        if (root == null) {
            invoke.reject("无法获取当前窗口根节点（rootInActiveWindow 为空）")
            return
        }
        val focus = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
        if (focus == null) {
            invoke.reject("当前界面没有输入焦点")
            return
        }
        val arguments = Bundle().apply {
            putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, text)
        }
        if (focus.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, arguments)) {
            invoke.resolve()
        } else {
            invoke.reject("文本输入失败：目标控件不接受 ACTION_SET_TEXT")
        }
    }

    // ---------- key（全局动作映射） ----------

    @Command
    fun key(invoke: Invoke) {
        val svc = serviceOrReject(invoke) ?: return
        val keyName = invoke.getArgs().getString("key")
        val action = globalActionForKey(keyName)
        if (action == null) {
            invoke.reject("不支持的按键：$keyName（支持 back/home/app_switch/recents/notifications/quick_settings/power/lock_screen，可带 KEYCODE_ 前缀）")
            return
        }
        if (svc.performGlobalAction(action)) {
            invoke.resolve()
        } else {
            invoke.reject("按键 $keyName 执行失败")
        }
    }

    private fun globalActionForKey(key: String): Int? {
        return when (key.removePrefix("KEYCODE_").lowercase()) {
            "back" -> AccessibilityService.GLOBAL_ACTION_BACK
            "home" -> AccessibilityService.GLOBAL_ACTION_HOME
            "app_switch", "recents" -> AccessibilityService.GLOBAL_ACTION_RECENTS
            "notifications" -> AccessibilityService.GLOBAL_ACTION_NOTIFICATIONS
            "quick_settings" -> AccessibilityService.GLOBAL_ACTION_QUICK_SETTINGS
            "power", "power_dialog" -> AccessibilityService.GLOBAL_ACTION_POWER_DIALOG
            "lock_screen" ->
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    AccessibilityService.GLOBAL_ACTION_LOCK_SCREEN
                } else {
                    null
                }

            else -> null
        }
    }

    // ---------- apps / launch ----------

    @Command
    fun apps(invoke: Invoke) {
        try {
            val args = invoke.getArgs()
            val filter = args.optString("filter", "").trim().lowercase()
            val all = args.optBoolean("all", false)
            val pm = activity.packageManager
            val packages = JSONArray()
            if (all) {
                val infos = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    pm.getInstalledPackages(PackageManager.PackageInfoFlags.of(0))
                } else {
                    @Suppress("DEPRECATION")
                    pm.getInstalledPackages(0)
                }
                infos
                    .map { it.packageName }
                    .distinct()
                    .sorted()
                    .filter { filter.isEmpty() || it.lowercase().contains(filter) }
                    .forEach { packages.put(it) }
            } else {
                val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                val resolved = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    pm.queryIntentActivities(intent, PackageManager.ResolveInfoFlags.of(0))
                } else {
                    @Suppress("DEPRECATION")
                    pm.queryIntentActivities(intent, 0)
                }
                resolved
                    .map { it.activityInfo.packageName to it.loadLabel(pm).toString() }
                    .distinctBy { it.first }
                    .sortedBy { it.first }
                    .filter {
                        filter.isEmpty() ||
                            it.first.lowercase().contains(filter) ||
                            it.second.lowercase().contains(filter)
                    }
                    .forEach { packages.put(it.first) }
            }
            invoke.resolve(JSObject().put("packages", packages))
        } catch (e: Exception) {
            invoke.reject("应用列表获取失败: ${e.message}")
        }
    }

    @Command
    fun launch(invoke: Invoke) {
        val pkg = invoke.getArgs().getString("package")
        val intent = activity.packageManager.getLaunchIntentForPackage(pkg)
        if (intent == null) {
            invoke.reject("应用不存在或没有启动入口: $pkg")
            return
        }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try {
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("启动应用失败: ${e.message}")
        }
    }

    // ---------- a11yStatus / openA11ySettings ----------

    @Command
    fun a11yStatus(invoke: Invoke) {
        invoke.resolve(JSObject().put("enabled", isServiceEnabled(activity)))
    }

    @Command
    fun openA11ySettings(invoke: Invoke) {
        try {
            val intent = Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("无法打开无障碍设置: ${e.message}")
        }
    }

    // ---------- 组 4：设备信息 / token 存储 / 前台服务 / 扫码 ----------

    /** 设备信息（hello 上报用）：Build.MODEL + Build.VERSION.RELEASE。 */
    @Command
    fun deviceInfo(invoke: Invoke) {
        invoke.resolve(
            JSObject()
                .put("device_name", Build.MODEL ?: "android")
                .put("android_version", Build.VERSION.RELEASE ?: "unknown"),
        )
    }

    /** 读取已配对 token（SharedPreferences 私有存储，键 `token:<host>:<port>`）。 */
    @Command
    fun getBridgeToken(invoke: Invoke) {
        val args = invoke.getArgs()
        val key = tokenKey(args.getString("host"), args.getInt("port"))
        val token = prefs().getString(key, null)
        invoke.resolve(JSObject().put("token", token))
    }

    /** 持久化配对成功签发的 token。 */
    @Command
    fun setBridgeToken(invoke: Invoke) {
        val args = invoke.getArgs()
        val key = tokenKey(args.getString("host"), args.getInt("port"))
        prefs().edit().putString(key, args.getString("token")).apply()
        invoke.resolve()
    }

    /** 删除已存 token（hello_ack 认证拒绝 = token 失效证据，Rust 侧调用）。 */
    @Command
    fun deleteBridgeToken(invoke: Invoke) {
        val args = invoke.getArgs()
        val key = tokenKey(args.getString("host"), args.getInt("port"))
        prefs().edit().remove(key).apply()
        invoke.resolve()
    }

    /** 读取上次成功连接的 daemon 地址（冷启动自动连接 + 连接页回填）。 */
    @Command
    fun getLastAddress(invoke: Invoke) {
        val host = prefs().getString("last_host", null)
        val port = prefs().getInt("last_port", 0)
        if (host.isNullOrEmpty() || port <= 0) {
            invoke.resolve(JSObject().put("host", JSONObject.NULL).put("port", 0))
        } else {
            invoke.resolve(JSObject().put("host", host).put("port", port))
        }
    }

    /** 持久化上次成功连接的 daemon 地址（仅连接成功后由 Rust 调用）。 */
    @Command
    fun setLastAddress(invoke: Invoke) {
        val args = invoke.getArgs()
        prefs().edit()
            .putString("last_host", args.getString("host"))
            .putInt("last_port", args.getInt("port"))
            .apply()
        invoke.resolve()
    }

    /** 启动前台服务保活（常驻通知「Agent Mobile Bridge 运行中」）。 */
    @Command
    fun startForegroundService(invoke: Invoke) {
        try {
            BridgeForegroundService.start(activity)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject("前台服务启动失败: ${e.message}")
        }
    }

    @Command
    fun stopForegroundService(invoke: Invoke) {
        BridgeForegroundService.stop(activity)
        invoke.resolve()
    }

    /** 打开扫码页扫描配对二维码，回传扫描内容文本。 */
    @Command
    fun scanPairQr(invoke: Invoke) {
        val started = ScanActivity.beginScan { text ->
            if (text == null) {
                invoke.reject("扫码取消或相机权限被拒绝")
            } else {
                invoke.resolve(JSObject().put("text", text))
            }
        }
        if (!started) {
            invoke.reject("已有扫码进行中")
            return
        }
        try {
            activity.startActivity(Intent(activity, ScanActivity::class.java))
        } catch (e: Exception) {
            ScanActivity.cancelScan()
            invoke.reject("无法打开扫码页: ${e.message}")
        }
    }

    private fun prefs() =
        activity.getSharedPreferences("agent_mobile_bridge", Context.MODE_PRIVATE)

    private fun tokenKey(host: String, port: Int) = "token:$host:$port"

    companion object {
        private const val TAP_DURATION_MS = 80L
        private const val MAX_TREE_DEPTH = 64

        /** 解析 Settings.Secure enabled_accessibility_services，判断本服务是否已启用。 */
        fun isServiceEnabled(context: Context): Boolean {
            val masterOn = Settings.Secure.getInt(
                context.contentResolver,
                Settings.Secure.ACCESSIBILITY_ENABLED,
                0,
            ) == 1
            if (!masterOn) {
                return false
            }
            val services = Settings.Secure.getString(
                context.contentResolver,
                Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES,
            ) ?: return false
            val expected = ComponentName(context, BridgeAccessibilityService::class.java)
            val fullName = expected.flattenToString()
            val shortName = expected.flattenToShortString()
            return services.split(':').any {
                it.equals(fullName, ignoreCase = true) || it.equals(shortName, ignoreCase = true)
            }
        }
    }
}
