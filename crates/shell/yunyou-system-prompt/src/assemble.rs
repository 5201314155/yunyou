//! 装配：compose 消费段注册表（契约③，对标官方 `assemble` waterfall + `renderPrompt`，
//! system-prompt/src/index.ts:56-79/:278-302）。
//!
//! - 段位表：按 `(order, name)` 升序拼接，相等段序按段名字典序（对标官方 :58-61）；
//! - 同名 shadow：并入时**后注册者覆盖**（见 `crate::registry` 同名语义）；
//! - 互斥段：`complete` 段生效时其余段（含工具段）让位，多个同时生效＝装配失败
//!   （对标官方 :70-74「restores this exact section as the sole prompt section」）；
//! - 工具段：每轮 schema 由组装调用方传入，是**宿主动态段**——段名归宿主、不写入注册表，
//!   但同台参与段位表排序（它是提示词文本，不是工具声明本体；声明本身由 agent-loop 下发）。

use crate::error::SystemPromptError;
use crate::interpolate::interpolate;
use crate::registry::SectionRegistry;
use crate::section::{Section, ORDER_PERSONA_PREFIX, ORDER_TOOLS, SECTION_PERSONA_PREFIX, SECTION_TOOLS};
use std::collections::{BTreeMap, HashMap};
use yunyou_llm::ToolSchema;

/// 工具段头部（存量文案，目录改名批次品牌口径已修）。
const TOOLS_HEADER: &str =
    "## 可用工具\n需要时调用工具完成操作；调用前先向用户说明你要做什么。";
/// 无工具时的明确说明（模型不会臆造工具）。
const TOOLS_EMPTY: &str = "当前没有可用工具，仅以文本回答。";

impl SectionRegistry {
    /// 契约③ 装配（可失败版）：段位表排序拼接 + complete 互斥校验。
    ///
    /// `persona` 是运行期人设槽替换：非空即覆盖注册表里的人设段（同名 shadow），
    /// 空则保留组件默认注册的人设段（默认段见 `crate::DefaultSystemPrompt`）。
    pub fn try_compose(
        &self,
        persona: &str,
        tools: &[ToolSchema],
    ) -> Result<String, SystemPromptError> {
        let override_personas = persona_sections(persona)?;
        self.assemble(&override_personas, tools, false)
    }

    /// 降级装配（不可失败版，`SystemPrompt::compose` 入口用）。
    ///
    /// 人设段畸形则跳过该段、complete 冲突则忽略互斥标记，两路均记 `error!` 日志——
    /// turn 热路径不因单段问题整体失败，但问题必须留痕（对标官方 renderPrompt 抛错；
    /// yunyou 因 `compose -> String` 签名冻结（消费方在辖区外）改走显式降级，见
    /// `crate::interpolate` 模块文档的同类裁断）。
    pub fn compose_lossy(&self, persona: &str, tools: &[ToolSchema]) -> String {
        let override_personas = match persona_sections(persona) {
            Ok(sections) => sections,
            Err(err) => {
                tracing::error!("人设段注册失败，本轮跳过人设: {err}");
                Vec::new()
            }
        };
        match self.assemble(&override_personas, tools, true) {
            Ok(text) => text,
            Err(err) => {
                tracing::error!("提示词装配失败，返回空提示词: {err}");
                String::new()
            }
        }
    }

    /// 装配内核：`personas` 为人设槽替换（可多段，md 标题切段）；`ignore_complete`
    /// 关闭互斥校验（降级路径）。
    fn assemble(
        &self,
        personas: &[Section],
        tools: &[ToolSchema],
        ignore_complete: bool,
    ) -> Result<String, SystemPromptError> {
        let mut effective: BTreeMap<String, Section> = BTreeMap::new();
        for section in self.records() {
            effective.insert(section.name.clone(), section);
        }
        for section in personas {
            effective.insert(section.name.clone(), section.clone());
        }
        effective.insert(
            SECTION_TOOLS.to_string(),
            Section::new(SECTION_TOOLS, ORDER_TOOLS, tools_text(tools)),
        );
        let complete: Vec<String> = effective
            .values()
            .filter(|section| section.complete)
            .map(|section| section.name.clone())
            .collect();
        if !ignore_complete && complete.len() > 1 {
            return Err(SystemPromptError::CompleteConflict { names: complete });
        }
        let mut sections: Vec<Section> = effective.into_values().collect();
        if complete.len() == 1 {
            sections.retain(|section| section.complete);
        }
        sections.sort_by(|a, b| {
            a.order.total_cmp(&b.order).then_with(|| a.name.cmp(&b.name))
        });
        let variables = self.variables();
        let mut parts: Vec<String> = Vec::new();
        for section in &sections {
            push_section(&mut parts, section, &variables);
        }
        Ok(parts.join("\n\n"))
    }
}

/// 运行期人设槽替换：空人设＝不替换（保留默认段）；md `#` 标题切段（池 task-6，
/// 见 `crate::persona_md`）——每段同人设槽档位，段名加序后缀保文件顺序（段位表
/// 排序 `(order, name)` 升序，同序按段名字典序＝后缀数字序）；无标题纯文本＝
/// 单段原语义（toml 字符串人设向后兼容）。非法在 `validate` 抛错。
fn persona_sections(persona: &str) -> Result<Vec<Section>, SystemPromptError> {
    let trimmed = persona.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let chunks = crate::persona_md::split_md_sections(trimmed);
    let sections: Vec<Section> = chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let name = if index == 0 {
                SECTION_PERSONA_PREFIX.to_string()
            } else {
                format!("{SECTION_PERSONA_PREFIX}/{index}")
            };
            Section::new(name, ORDER_PERSONA_PREFIX, chunk)
        })
        .collect();
    for section in &sections {
        section.validate()?;
    }
    Ok(sections)
}

/// 渲染一个段：插值 + 空段（含纯空白）不贡献（对标官方「empty text contributes nothing」）。
fn push_section(parts: &mut Vec<String>, section: &Section, variables: &HashMap<String, String>) {
    let text = interpolate(&section.text, variables);
    let trimmed = text.trim_end();
    if !trimmed.trim().is_empty() {
        parts.push(trimmed.to_string());
    }
}

/// 合成工具动态段文本（宿主拥有，段名/档位见 `SECTION_TOOLS`/`ORDER_TOOLS`）。
fn tools_text(tools: &[ToolSchema]) -> String {
    if tools.is_empty() {
        return TOOLS_EMPTY.to_string();
    }
    let mut text = String::from(TOOLS_HEADER);
    for tool in tools {
        text.push_str(&format!("\n- {}: {}", tool.name, tool.description));
    }
    text
}