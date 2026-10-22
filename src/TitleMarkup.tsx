// Deliberately limited Markdown emphasis for task titles: agents can use
// **words** to highlight priority information without enabling HTML/links.
export function TitleMarkup({ title }: { title: string }) {
  return (
    <>
      {title.split(/(\*\*[^*\n]+\*\*)/g).map((part, index) =>
        part.startsWith("**") && part.endsWith("**") && part.length > 4
          ? <strong key={index}>{part.slice(2, -2)}</strong>
          : part,
      )}
    </>
  );
}
