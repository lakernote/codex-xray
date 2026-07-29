export default function BrandMark() {
  return (
    <svg
      className="brand-mark"
      viewBox="0 0 512 512"
      aria-hidden="true"
      focusable="false"
    >
      <rect
        className="brand-mark-surface"
        x="5"
        y="5"
        width="502"
        height="502"
        rx="111"
      />
      <path
        className="brand-mark-back"
        d="M256 228C266 228 275 231 284 236L371 287C384 295 384 309 371 317L284 368C267 378 245 378 228 368L141 317C128 309 128 295 141 287L228 236C237 231 246 228 256 228Z"
      />
      <path
        className="brand-mark-active"
        d="M256 170C266 170 275 173 284 178L371 229C384 237 384 251 371 259L284 310C267 320 245 320 228 310L141 259C128 251 128 237 141 229L228 178C237 173 246 170 256 170Z"
      />
      <path
        className="brand-mark-front"
        d="M256 112C266 112 275 115 284 120L371 171C384 179 384 193 371 201L284 252C267 262 245 262 228 252L141 201C128 193 128 179 141 171L228 120C237 115 246 112 256 112Z"
      />
      <path
        className="brand-mark-detail"
        d="M207 183L256 212L305 183"
      />
    </svg>
  );
}
