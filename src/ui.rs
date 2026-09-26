//! 屏幕 UI 快照：uiautomator XML 简化树与 @eN 元素引用（design.md 决策 4）。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ElemRef {
    /// 引用标识，如 "@e1"
    pub id: String,
    pub text: Option<String>,
    pub content_desc: Option<String>,
    pub class: Option<String>,
    /// 元素区域中心点（屏幕坐标）
    pub center: (i32, i32),
    /// 元素区域 [left,top,right,bottom]
    pub bounds: (i32, i32, i32, i32),
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    /// 简化后的缩进树文本（面向 Agent 阅读）
    pub tree: String,
    /// 可交互元素引用表
    pub refs: Vec<ElemRef>,
}

/// 将 uiautomator dump 的 XML 解析为简化快照。
/// 简化规则（design.md 决策 4）：剔除零面积节点与无文本无描述且不可交互的容器
/// （递归上提子节点）；可交互节点按先序分配 @eN。
pub fn simplify(xml: &str) -> Result<Snapshot, String> {
    let _ = xml;
    unimplemented!("任务 4.1 实现")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 样例：模拟设置页层级（含可点击项、纯容器、零面积节点）
    const SAMPLE: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][1080,2400]">
    <node index="0" text="" resource-id="android:id/content" class="android.widget.FrameLayout" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][1080,2400]">
      <node index="0" text="设置" resource-id="" class="android.widget.TextView" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[48,64][240,160]"/>
      <node index="1" text="" resource-id="" class="android.widget.LinearLayout" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,200][1080,400]">
        <node index="0" text="WLAN" resource-id="" class="android.widget.TextView" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[48,220][1032,380]"/>
      </node>
      <node index="2" text="蓝牙" resource-id="" class="android.widget.TextView" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[48,420][1032,580]"/>
      <node index="3" text="" resource-id="" class="android.view.View" package="com.android.settings" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][0,0]"/>
    </node>
  </node>
</hierarchy>"#;

    #[test]
    fn assigns_refs_in_document_order() {
        let snap = simplify(SAMPLE).unwrap();
        let ids: Vec<&str> = snap.refs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["@e1", "@e2"]);
        assert_eq!(snap.refs[0].text.as_deref(), Some("WLAN"));
        assert_eq!(snap.refs[1].text.as_deref(), Some("蓝牙"));
    }

    #[test]
    fn computes_center_from_bounds() {
        let snap = simplify(SAMPLE).unwrap();
        // [48,220][1032,380] → center (540, 300)
        assert_eq!(snap.refs[0].center, (540, 300));
    }

    #[test]
    fn drops_zero_area_and_empty_containers() {
        let snap = simplify(SAMPLE).unwrap();
        // 零面积节点不出现在树中；无内容容器被合并但保留有内容的结构
        assert!(!snap.tree.contains("android.view.View"));
        assert!(snap.tree.contains("WLAN"));
        assert!(snap.tree.contains("蓝牙"));
        // 标题文本节点保留在树里（无引用但可见）
        assert!(snap.tree.contains("设置"));
    }

    #[test]
    fn refs_appear_in_tree_with_marks() {
        let snap = simplify(SAMPLE).unwrap();
        assert!(snap.tree.contains("@e1"));
        assert!(snap.tree.contains("@e2"));
    }

    #[test]
    fn rejects_invalid_xml() {
        assert!(simplify("<not-closed").is_err());
    }
}
