import { Modal } from '@/components/Modal';
import type { PendingChange } from '@/lib/ipc';
import { forgetChange } from '@/state/recovery';
import { fileName, forgetLines, pendingReason } from './text';

const BUTTON =
  'inline-flex h-[30px] items-center whitespace-nowrap rounded-[7px] border px-3 text-[13px] font-medium';

/** Says what Forget means before it is done: the change is not tried again, and nothing is deleted. */
export function ForgetConfirm({ change, onClose }: { change: PendingChange; onClose: () => void }) {
  const name = fileName(change.path);
  return (
    <Modal label={`Forget the change to ${name}?`} onClose={onClose}>
      <div className="flex min-h-0 flex-col gap-2 overflow-y-auto px-5 pb-4 pt-[18px] text-[13px] leading-[1.45]">
        <h2 className="text-[15px] font-semibold text-fg-0">{`Forget the change to ${name}?`}</h2>
        <p className="break-all font-mono text-[12px] text-fg-2">{change.path}</p>
        <p className="text-fg-2">{`${pendingReason(change.reason)}.`}</p>
        {forgetLines(change).map((line) => (
          <p key={line} className="text-fg-1">
            {line}
          </p>
        ))}
      </div>
      <div className="flex shrink-0 items-center justify-end gap-2 border-t border-line px-5 py-3">
        <button
          type="button"
          onClick={onClose}
          className={`${BUTTON} border-line text-fg-0 hover:bg-bg-2`}
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={() => {
            onClose();
            void forgetChange(change.txn);
          }}
          className={`${BUTTON} border-transparent bg-accent text-bg-0`}
        >
          Forget
        </button>
      </div>
    </Modal>
  );
}
