import { FilesDropzone } from "./FilesDropzone";

export interface ModeFilesDropzoneProps {
  modeId: string;
  onAdded: () => void;
}

/** Modes → Files: the shared dropzone bound to one mode's scope. */
export function ModeFilesDropzone({ modeId, onAdded }: ModeFilesDropzoneProps) {
  return <FilesDropzone kind="notes" scope="mode" scopeId={modeId} onAdded={onAdded} />;
}
