//! Feishu card builder — constructs cards from plan data.

use super::elements::{ButtonElement, ButtonStyle, CardAction, CardElement, MarkdownElement, ProgressElement};
use super::{CardHeader, PlanData, PlanStep, RichCard, StepStatus};

/// Feishu card builder for constructing Plan mode cards.
pub struct FeishuCardBuilder;

impl FeishuCardBuilder {
    /// Build a complete Plan mode card from plan data.
    pub fn build_plan_card(plan: &PlanData) -> RichCard {
        let mut elements = Vec::new();

        // Progress bar (always at the top)
        elements.push(CardElement::Progress(ProgressElement {
            current: plan.current_step,
            total: plan.total_steps,
            labels: Some(plan.step_labels.clone()),
        }));

        // Divider after progress bar
        elements.push(CardElement::Divider);

        // Step contents
        for step in &plan.steps {
            elements.push(Self::format_step_content(step));
        }

        // Action buttons for high complexity tasks
        if plan.is_high_complexity {
            elements.push(CardElement::Divider);
            elements.push(CardElement::Button(ButtonElement {
                text: "✅ 确认计划".to_string(),
                action: CardAction::Confirm,
                style: ButtonStyle::Primary,
            }));
            elements.push(CardElement::Button(ButtonElement {
                text: "🔄 重新调整".to_string(),
                action: CardAction::Custom {
                    payload: "regenerate".to_string(),
                },
                style: ButtonStyle::Secondary,
            }));
        }

        RichCard {
            card_id: None,
            title: plan.title.clone(),
            elements,
            header: Some(CardHeader {
                title: plan.title.clone(),
                subtitle: Some(format!("步骤 {}/{}", plan.current_step, plan.total_steps)),
                avatar_url: None,
            }),
        }
    }

    /// Format a single step's content as a markdown element.
    fn format_step_content(step: &PlanStep) -> CardElement {
        let status_icon = match step.status {
            StepStatus::Pending => "⏳",
            StepStatus::Active => "🔄",
            StepStatus::Completed => "✅",
        };

        let content = format!(
            "{} **{}**\n\n{}",
            status_icon,
            step.title,
            step.content
        );

        CardElement::Markdown(MarkdownElement {
            content,
            collapsible: true,
            collapsed: step.status == StepStatus::Pending,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_plan() -> PlanData {
        PlanData {
            title: "测试计划".to_string(),
            current_step: 2,
            total_steps: 3,
            step_labels: vec!["需求分析".to_string(), "方案设计".to_string(), "实现".to_string()],
            steps: vec![
                PlanStep {
                    title: "需求分析".to_string(),
                    content: "分析用户需求".to_string(),
                    status: StepStatus::Completed,
                },
                PlanStep {
                    title: "方案设计".to_string(),
                    content: "设计技术方案".to_string(),
                    status: StepStatus::Active,
                },
                PlanStep {
                    title: "实现".to_string(),
                    content: "编写代码".to_string(),
                    status: StepStatus::Pending,
                },
            ],
            is_high_complexity: true,
        }
    }

    #[test]
    fn test_build_plan_card_basic_structure() {
        let plan = make_test_plan();
        let card = FeishuCardBuilder::build_plan_card(&plan);

        assert_eq!(card.title, "测试计划");
        assert!(card.header.is_some());
        assert_eq!(card.header.as_ref().unwrap().title, "测试计划");
        assert_eq!(
            card.header.as_ref().unwrap().subtitle,
            Some("步骤 2/3".to_string())
        );
    }

    #[test]
    fn test_build_plan_card_elements_count() {
        let plan = make_test_plan();
        let card = FeishuCardBuilder::build_plan_card(&plan);

        // Progress + Divider + 3 steps + Divider + 2 buttons = 8 elements
        assert_eq!(card.elements.len(), 8);
    }

    #[test]
    fn test_build_plan_card_first_element_is_progress() {
        let plan = make_test_plan();
        let card = FeishuCardBuilder::build_plan_card(&plan);

        match &card.elements[0] {
            CardElement::Progress(el) => {
                assert_eq!(el.current, 2);
                assert_eq!(el.total, 3);
            }
            _ => panic!("First element should be Progress"),
        }
    }

    #[test]
    fn test_build_plan_card_step_content() {
        let plan = make_test_plan();
        let card = FeishuCardBuilder::build_plan_card(&plan);

        // elements[2] is the first step (Completed, after progress + divider)
        match &card.elements[2] {
            CardElement::Markdown(el) => {
                assert!(el.content.contains("✅"));
                assert!(el.content.contains("需求分析"));
                assert!(el.collapsible);
                assert!(!el.collapsed); // Completed steps are expanded
            }
            _ => panic!("Step should be Markdown"),
        }

        // elements[4] is the third step (Pending)
        match &card.elements[4] {
            CardElement::Markdown(el) => {
                assert!(el.content.contains("⏳"));
                assert!(el.content.contains("实现"));
                assert!(el.collapsible);
                assert!(el.collapsed); // Pending steps are collapsed
            }
            _ => panic!("Step should be Markdown"),
        }
    }

    #[test]
    fn test_build_plan_card_buttons_present() {
        let plan = make_test_plan();
        let card = FeishuCardBuilder::build_plan_card(&plan);

        // Last two elements should be buttons
        match &card.elements[6] {
            CardElement::Button(el) => {
                assert_eq!(el.text, "✅ 确认计划");
                assert!(matches!(el.action, CardAction::Confirm));
                assert_eq!(el.style, ButtonStyle::Primary);
            }
            _ => panic!("Should have Confirm button"),
        }

        // Verify the regenerate button's action matches correctly
        let regenerate_button = match &card.elements[7] {
            CardElement::Button(el) => el,
            _ => panic!("Should have Regenerate button"),
        };
        match &regenerate_button.action {
            CardAction::Custom { payload } => {
                assert_eq!(payload, "regenerate");
            }
            _ => panic!("Regenerate button should have Custom action"),
        }
        assert_eq!(regenerate_button.style, ButtonStyle::Secondary);
    }

    #[test]
    fn test_build_plan_card_low_complexity_no_buttons() {
        let mut plan = make_test_plan();
        plan.is_high_complexity = false;

        let card = FeishuCardBuilder::build_plan_card(&plan);

        // Progress + Divider + 3 steps = 5 elements (no buttons)
        assert_eq!(card.elements.len(), 5);

        // No button elements
        for el in &card.elements {
            assert!(!matches!(el, CardElement::Button(_)));
        }
    }

    #[test]
    fn test_format_step_content_pending() {
        let step = PlanStep {
            title: "待处理步骤".to_string(),
            content: "内容".to_string(),
            status: StepStatus::Pending,
        };

        let markdown = FeishuCardBuilder::format_step_content(&step);
        match markdown {
            CardElement::Markdown(el) => {
                assert!(el.content.contains("⏳"));
                assert!(el.content.contains("待处理步骤"));
                assert!(el.collapsed);
            }
            _ => panic!("Should be Markdown"),
        }
    }

    #[test]
    fn test_format_step_content_active() {
        let step = PlanStep {
            title: "进行中".to_string(),
            content: "正在处理...".to_string(),
            status: StepStatus::Active,
        };

        let markdown = FeishuCardBuilder::format_step_content(&step);
        match markdown {
            CardElement::Markdown(el) => {
                assert!(el.content.contains("🔄"));
                assert!(!el.collapsed); // Active step not collapsed
            }
            _ => panic!("Should be Markdown"),
        }
    }

    #[test]
    fn test_format_step_content_completed() {
        let step = PlanStep {
            title: "已完成".to_string(),
            content: "完成内容".to_string(),
            status: StepStatus::Completed,
        };

        let markdown = FeishuCardBuilder::format_step_content(&step);
        match markdown {
            CardElement::Markdown(el) => {
                assert!(el.content.contains("✅"));
                assert!(!el.collapsed);
            }
            _ => panic!("Should be Markdown"),
        }
    }
}
