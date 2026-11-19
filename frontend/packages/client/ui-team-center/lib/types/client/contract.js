/** 团队中心数据形状与远程面接口缝（web.v1 team/tasks 域，域细则 §1–§2）。 */
/** 共享任务状态机（board.rs 冻结）：rejected 为终态，重做=新建。 */
export const TASK_STATES = ['pending', 'claimed', 'submitted', 'reviewing', 'approved', 'rejected'];
//# sourceMappingURL=contract.js.map