package com.agentmobile.bridge

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import com.journeyapps.barcodescanner.BarcodeCallback
import com.journeyapps.barcodescanner.BarcodeResult
import com.journeyapps.barcodescanner.DecoratedBarcodeView

/**
 * 扫码页（zxing）：扫描 daemon `pair` 输出的配对二维码
 * （内容 `agent-mobile://pair?host=..&port=..&code=..`），
 * 结果经 [pendingResult] 回传给 BridgePlugin 的 scanPairQr 命令。
 *
 * 相机权限流程：进入时检查 CAMERA；未授权则运行时申请，授权后继续，拒绝则取消回传。
 */
class ScanActivity : Activity() {

    private var barcodeView: DecoratedBarcodeView? = null
    private var delivered = false

    private val callback = BarcodeCallback { result: BarcodeResult ->
        deliver(result.text)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (hasCameraPermission()) {
            initScanner()
        } else {
            ActivityCompat.requestPermissions(
                this,
                arrayOf(Manifest.permission.CAMERA),
                REQUEST_CAMERA,
            )
        }
    }

    private fun hasCameraPermission(): Boolean {
        return ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
    }

    private fun initScanner() {
        val view = DecoratedBarcodeView(this)
        view.decodeContinuous(callback)
        barcodeView = view
        setContentView(view)
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == REQUEST_CAMERA) {
            if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
                initScanner()
            } else {
                deliver(null)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        barcodeView?.resume()
    }

    override fun onPause() {
        barcodeView?.pause()
        super.onPause()
    }

    override fun onDestroy() {
        // 返回键/系统回收：取消回传，避免 Rust 侧永久阻塞
        if (!delivered) {
            deliver(null)
        }
        super.onDestroy()
    }

    private fun deliver(text: String?) {
        if (delivered) {
            return
        }
        delivered = true
        val callback = pendingResult
        pendingResult = null
        callback?.invoke(text)
        finish()
    }

    companion object {
        private const val REQUEST_CAMERA = 41

        @Volatile
        private var pendingResult: ((String?) -> Unit)? = null

        /** 登记一次扫码回调；已有进行中的扫码返回 false。 */
        fun beginScan(callback: (String?) -> Unit): Boolean {
            synchronized(this) {
                if (pendingResult != null) {
                    return false
                }
                pendingResult = callback
                return true
            }
        }

        /** 扫码页未能启动时清除登记，使后续扫码可用。 */
        fun cancelScan() {
            synchronized(this) {
                pendingResult = null
            }
        }
    }
}
