#include <gif_lib.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void handle_gif_error(const char *error_msg) {
  fprintf(stderr, "Error: %s\n", error_msg);
  exit(EXIT_FAILURE);
}

void print_gif_info(const char *filename) {
  GifFileType *gif = DGifOpenFileName(filename, NULL);

  if (!gif) {
    handle_gif_error("Unable to open GIF file");
  }

  printf("GIF Information for: %s\n", filename);
  printf("Number of frames: %d\n", gif->ImageCount);
  printf("Width: %d pixels\n", gif->SWidth);
  printf("Height: %d pixels\n", gif->SHeight);

  DGifCloseFile(gif, NULL);
}

int main(int argc, char *argv[]) {
  if (argc != 2) {
    fprintf(stderr, "Usage: %s <gif_file>\n", argv[0]);
    return EXIT_FAILURE;
  }

  const char *filename = argv[1];
  print_gif_info(filename);

  return EXIT_SUCCESS;
}
