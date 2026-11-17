/** A stored image readable from the program's execution environment. */
export class PtcImage {
    /** Absolute attachment file path readable in the execution world. */
    path;
    /** Stored image selected when the program inspects this value. */
    attachment;
    /**
     * Describe one immutable image file in the execution world.
     * @param path - Absolute path to the stored image.
     * @param attachment - Reference to the immutable stored image.
     */
    constructor(path, attachment) {
        this.path = path;
        this.attachment = attachment;
        Object.freeze(this);
    }
}
//# sourceMappingURL=image.js.map