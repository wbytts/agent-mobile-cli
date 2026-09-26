//! 屏幕 UI 快照：uiautomator XML 简化树与 @eN 元素引用（design.md 决策 4）。

use quick_xml::events::Event;
use quick_xml::Reader;
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

/// 解析阶段的原始节点（含被简化剔除的容器）。
#[derive(Debug, Default)]
struct RawNode {
    class: String,
    text: String,
    desc: String,
    /// clickable/long-clickable/focusable/scrollable 任一为真
    interactive: bool,
    bounds: (i32, i32, i32, i32),
    children: Vec<RawNode>,
}

/// 将 uiautomator dump 的 XML 解析为简化快照。
/// 简化规则（design.md 决策 4）：剔除零面积节点与无文本无描述且不可交互的容器
/// （递归上提子节点）；可交互节点按先序分配 @eN。
pub fn simplify(xml: &str) -> Result<Snapshot, String> {
    let roots = parse_nodes(xml)?;
    let mut lines = Vec::new();
    let mut refs = Vec::new();
    for node in &roots {
        render_node(node, 0, &mut lines, &mut refs);
    }
    Ok(Snapshot {
        tree: lines.join("\n"),
        refs,
    })
}

/// 事件流解析 <node> 元素为树；标签不闭合、属性或 bounds 非法均报错。
fn parse_nodes(xml: &str) -> Result<Vec<RawNode>, String> {
    let mut reader = Reader::from_str(xml);
    let mut roots: Vec<RawNode> = Vec::new();
    let mut stack: Vec<RawNode> = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("XML 解析失败: {e}"))?
        {
            Event::Start(e) if e.name().as_ref() == b"node" => {
                stack.push(parse_node_attrs(&reader, &e)?);
            }
            Event::Empty(e) if e.name().as_ref() == b"node" => {
                let node = parse_node_attrs(&reader, &e)?;
                attach_node(node, &mut stack, &mut roots);
            }
            Event::End(e) if e.name().as_ref() == b"node" => {
                let node = stack
                    .pop()
                    .ok_or_else(|| "XML 结构错误：存在多余的 </node>".to_string())?;
                attach_node(node, &mut stack, &mut roots);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !stack.is_empty() {
        return Err("XML 结构错误：存在未闭合的 <node>".to_string());
    }
    Ok(roots)
}

fn attach_node(node: RawNode, stack: &mut [RawNode], roots: &mut Vec<RawNode>) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(node),
        None => roots.push(node),
    }
}

fn parse_node_attrs(
    reader: &Reader<&[u8]>,
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<RawNode, String> {
    let mut node = RawNode::default();
    for attr in e.attributes() {
        let attr = attr.map_err(|e| format!("属性解析失败: {e}"))?;
        let value = attr
            .decode_and_unescape_value(reader.decoder())
            .map_err(|e| format!("属性值解码失败: {e}"))?
            .into_owned();
        match attr.key.as_ref() {
            b"class" => node.class = value,
            b"text" => node.text = value,
            b"content-desc" => node.desc = value,
            b"bounds" => node.bounds = parse_bounds(&value)?,
            b"clickable" | b"long-clickable" | b"focusable" | b"scrollable" => {
                node.interactive |= value == "true";
            }
            _ => {}
        }
    }
    Ok(node)
}

/// 解析 "[l,t][r,b]" 形式的 bounds。
fn parse_bounds(s: &str) -> Result<(i32, i32, i32, i32), String> {
    let err = || format!("无法解析 bounds: {s}");
    let inner = s
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or_else(err)?;
    let (lt, rb) = inner.split_once("][").ok_or_else(err)?;
    let pair = |p: &str| -> Result<(i32, i32), String> {
        let (a, b) = p.split_once(',').ok_or_else(err)?;
        Ok((
            a.trim().parse().map_err(|_| err())?,
            b.trim().parse().map_err(|_| err())?,
        ))
    };
    let (l, t) = pair(lt)?;
    let (r, b) = pair(rb)?;
    Ok((l, t, r, b))
}

/// 递归渲染简化树：零面积节点整棵剔除；无内容且不可交互的容器不占行，
/// 子节点上提到该容器的缩进层级；保留下来的节点子级缩进 +1。
fn render_node(node: &RawNode, depth: usize, lines: &mut Vec<String>, refs: &mut Vec<ElemRef>) {
    let (l, t, r, b) = node.bounds;
    if r <= l || b <= t {
        return;
    }
    let has_content = !node.text.is_empty() || !node.desc.is_empty();
    let mut child_depth = depth;
    if node.interactive || has_content {
        let short_class = node.class.rsplit('.').next().unwrap_or(&node.class);
        let mut line = format!("{}{}", "  ".repeat(depth), short_class);
        if !node.text.is_empty() {
            line.push_str(&format!(" \"{}\"", node.text));
        }
        if !node.desc.is_empty() {
            line.push_str(&format!(" desc:\"{}\"", node.desc));
        }
        if node.interactive {
            let id = format!("@e{}", refs.len() + 1);
            refs.push(ElemRef {
                id: id.clone(),
                text: non_empty(&node.text),
                content_desc: non_empty(&node.desc),
                class: non_empty(&node.class),
                center: ((l + r) / 2, (t + b) / 2),
                bounds: node.bounds,
            });
            line.push_str(&format!(" {id}"));
        }
        line.push_str(&format!(" [{l},{t}][{r},{b}]"));
        lines.push(line);
        child_depth = depth + 1;
    }
    for child in &node.children {
        render_node(child, child_depth, lines, refs);
    }
}

fn non_empty(s: &str) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
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

    /// 样例：可滚动容器内嵌无内容 LinearLayout，包裹一个按钮
    const NESTED: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.ScrollView" package="com.x" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="true" long-clickable="false" password="false" selected="false" bounds="[0,0][1080,2400]">
    <node index="0" text="" resource-id="" class="android.widget.LinearLayout" package="com.x" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,100][1080,300]">
      <node index="0" text="确定" resource-id="" class="android.widget.Button" package="com.x" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[400,150][680,250]"/>
    </node>
  </node>
</hierarchy>"#;

    #[test]
    fn lifts_button_out_of_empty_container_with_correct_indent() {
        let snap = simplify(NESTED).unwrap();
        // 无内容的 LinearLayout 不单独占行
        assert!(!snap.tree.contains("LinearLayout"));
        let lines: Vec<&str> = snap.tree.lines().collect();
        assert_eq!(lines.len(), 2, "树应为两行：{}", snap.tree);
        // ScrollView 有引用，位于层级 0；按钮上提到 LinearLayout 的位置，即 ScrollView 之下一级
        assert!(lines[0].starts_with("ScrollView"), "首行: {}", lines[0]);
        assert!(
            lines[1].starts_with("  Button \"确定\""),
            "次行: {}",
            lines[1]
        );
    }

    #[test]
    fn scrollable_container_gets_ref() {
        let snap = simplify(NESTED).unwrap();
        let ids: Vec<&str> = snap.refs.iter().map(|r| r.id.as_str()).collect();
        // 先序：ScrollView @e1，按钮 @e2
        assert_eq!(ids, ["@e1", "@e2"]);
        assert_eq!(
            snap.refs[0].class.as_deref(),
            Some("android.widget.ScrollView")
        );
        assert!(snap.tree.lines().next().unwrap().contains("@e1"));
    }

    #[test]
    fn shows_content_desc_and_ref() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.ImageButton" package="com.x" content-desc="更多选项" checkable="false" checked="false" clickable="true" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[900,50][1030,150]"/>
</hierarchy>"#;
        let snap = simplify(xml).unwrap();
        assert_eq!(snap.refs.len(), 1);
        assert_eq!(snap.refs[0].content_desc.as_deref(), Some("更多选项"));
        assert_eq!(snap.refs[0].text, None);
        // content-desc 展示在树行中
        assert!(snap.tree.contains("更多选项"), "树: {}", snap.tree);
        assert!(snap.tree.contains("@e1"));
    }
}
